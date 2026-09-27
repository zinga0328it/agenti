//! Logica di watchdog: stato di salute di Ollama, limitatore di restart e
//! macchina a stati per la protezione termica della GPU. Le strutture qui
//! dentro sono pure (nessuna I/O) per essere testabili deterministicamente;
//! l'orchestrazione con I/O reale (HTTP, systemctl, nvidia-smi) vive nelle
//! funzioni `run_*` in fondo al file e nel resto dei moduli.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::gpu::{classify_temperature, read_gpu_stats, ThermalLevel};
use crate::ollama::{check_health, wait_until_reachable};
use crate::process;
use crate::status::SharedStatus;

/// Contatore dei fallimenti consecutivi dell'health check.
#[derive(Debug, Clone)]
pub struct HealthMonitor {
    consecutive_failures: u32,
    max_failures: u32,
}

impl HealthMonitor {
    pub fn new(max_failures: u32) -> Self {
        Self {
            consecutive_failures: 0,
            max_failures,
        }
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Registra un fallimento. Ritorna `true` se la soglia è stata
    /// raggiunta/superata, cioè se va innescato il restart forzato.
    pub fn record_failure(&mut self) -> bool {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.consecutive_failures >= self.max_failures
    }

    /// Registra un successo, azzerando il contatore.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }
}

/// Limita il numero di restart forzati in una finestra temporale scorrevole.
#[derive(Debug, Clone)]
pub struct RestartLimiter {
    timestamps: VecDeque<Instant>,
    window: Duration,
    max_restarts: u32,
}

impl RestartLimiter {
    pub fn new(window: Duration, max_restarts: u32) -> Self {
        Self {
            timestamps: VecDeque::new(),
            window,
            max_restarts,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&front) = self.timestamps.front() {
            if now.duration_since(front) > self.window {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }

    /// Numero di restart registrati e ancora dentro la finestra.
    pub fn restarts_in_window(&mut self, now: Instant) -> usize {
        self.prune(now);
        self.timestamps.len()
    }

    /// Prova a registrare un nuovo restart. Ritorna `true` se consentito
    /// (e lo registra), `false` se il limite è già stato raggiunto: in quel
    /// caso il chiamante deve entrare in stato FAULT e NON procedere.
    pub fn try_record_restart(&mut self, now: Instant) -> bool {
        self.prune(now);
        if self.timestamps.len() as u32 >= self.max_restarts {
            false
        } else {
            self.timestamps.push_back(now);
            true
        }
    }
}

/// Stato globale del watchdog verso Ollama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardState {
    Normal,
    /// Troppi restart in finestra: il watchdog smette di intervenire e
    /// richiede attenzione umana.
    Fault(String),
    /// Ollama è stato fermato per temperatura critica e resterà fermo
    /// finché la temperatura non torna sotto la soglia di ripristino.
    ThermalHold,
}

/// Azione derivata dall'osservazione della temperatura GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuAction {
    Nothing,
    LogWarning,
    StopCritical,
    StillTooHotToResume,
    ResumeAllowed,
}

/// Macchina a stati per la protezione termica: ricorda se Ollama è stato
/// fermato a causa della temperatura, per evitare di riavviarlo finché non
/// si torna sotto la soglia di sicurezza configurata.
#[derive(Debug, Clone, Default)]
pub struct GpuThermalGuard {
    stopped_for_thermal: bool,
}

impl GpuThermalGuard {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn is_holding(&self) -> bool {
        self.stopped_for_thermal
    }

    pub fn evaluate(
        &mut self,
        temperature_c: u32,
        warning_threshold: u32,
        critical_threshold: u32,
        resume_threshold: u32,
    ) -> GpuAction {
        if self.stopped_for_thermal {
            return if temperature_c < resume_threshold {
                self.stopped_for_thermal = false;
                GpuAction::ResumeAllowed
            } else {
                GpuAction::StillTooHotToResume
            };
        }

        match classify_temperature(temperature_c, warning_threshold, critical_threshold) {
            ThermalLevel::Normal => GpuAction::Nothing,
            ThermalLevel::Warning => GpuAction::LogWarning,
            ThermalLevel::Critical => {
                self.stopped_for_thermal = true;
                GpuAction::StopCritical
            }
        }
    }
}

/// Chiede a systemd di riavviare Ollama e verifica che sia REALMENTE
/// tornato sano, invece di fidarsi del codice di uscita della CLI.
///
/// ARCHITETTURA (revisione del 2026-09-27): non reimplementiamo più la
/// logica di escalation SIGTERM→SIGKILL qui. Quella responsabilità è stata
/// spostata nell'unit `ollama.service` stessa, configurata con
/// `TimeoutStopSec` breve, `KillMode=control-group` e `SendSIGKILL=yes`: se
/// il processo non risponde al SIGTERM entro il timeout dell'unit, è
/// systemd a forzare il SIGKILL sull'intero control-group. Questo watchdog
/// si limita a:
/// 1. chiedere `systemctl restart` (un solo job atomico stop+start);
/// 2. verificare con polling reale (`wait_until_reachable`, HTTP verso
///    `/api/tags`) che Ollama sia di nuovo raggiungibile entro una
///    deadline bounded.
///
/// Non fidarsi mai del tempo di ritorno della CLI amministrativa: se
/// `systemctl restart` impiega più del previsto (o il nostro `await` viene
/// abbandonato per qualunque motivo), il comando continua comunque a girare
/// in background (nessun `kill_on_drop` sul processo client, vedi
/// `process.rs`) e la vera fonte di verità resta la risposta HTTP reale.
///
/// STORIA: la versione precedente di questa funzione reimplementava a mano
/// stop con timeout breve + kill con timeout breve + polling di
/// `pgrep`. Durante il test end-to-end reale con Ollama congelato via
/// SIGSTOP sono emersi due bug (timeout troppo corti per `systemctl kill`,
/// che su questa macchina può impiegare fino a ~14s, e un
/// `kill_on_drop(true)` che annullava il tentativo di kill prima che
/// venisse consegnato). Delegare l'intera escalation a systemd elimina
/// quella classe di bug alla radice, come richiesto: "non reinventare la
/// gestione dei processi che systemd sa già fare".
pub async fn force_restart_ollama(config: &Config) -> Result<(), String> {
    log::warn!("Asking systemd to restart Ollama (systemctl restart)");
    if let Err(e) = process::restart_ollama().await {
        // Non interrompiamo subito: anche se la CLI riporta un errore (o
        // impiega tempo), la vera verifica è la raggiungibilità HTTP qui
        // sotto. Un errore qui viene comunque loggato per diagnosi.
        log::warn!("systemctl restart reported an error (will still verify real health): {e}");
    }

    let ready = wait_until_reachable(
        &config.ollama.url,
        Duration::from_millis(500),
        Duration::from_secs(config.restart.recovery_timeout_seconds),
    )
    .await;

    if ready {
        log::info!("Ollama restarted successfully");
        Ok(())
    } else {
        log::error!("Ollama did not become reachable after restart");
        Err("ollama unreachable after restart".to_string())
    }
}

/// Loop di health-check continuo verso Ollama, indipendente da Ollama
/// stesso (usiamo solo HTTP con timeout stretto verso localhost).
pub async fn run_health_watchdog(config: Config, status: SharedStatus) {
    let mut health = HealthMonitor::new(config.ollama.max_health_failures);
    let mut limiter = RestartLimiter::new(
        Duration::from_secs(config.restart.restart_window_seconds),
        config.restart.max_restarts_per_window,
    );
    let mut state = GuardState::Normal;

    let interval = Duration::from_secs(config.ollama.health_interval_seconds);
    let timeout = Duration::from_secs(config.ollama.health_timeout_seconds);

    loop {
        tokio::time::sleep(interval).await;

        if let GuardState::Fault(reason) = &state {
            log::error!("FAULT state active, watchdog is not intervening: {reason}");
            continue;
        }
        if state == GuardState::ThermalHold {
            // La GPU watchdog governa il rientro da questo stato.
            continue;
        }

        match check_health(&config.ollama.url, timeout).await {
            Ok(()) => {
                if health.consecutive_failures() > 0 {
                    log::info!("Ollama healthy again");
                }
                health.record_success();
                log::info!("Ollama healthy");
                status.record_ollama_health(true, None);
                status.record_restarts_in_window(limiter.restarts_in_window(Instant::now()) as u32);
            }
            Err(e) => {
                let should_restart = health.record_failure();
                log::warn!(
                    "Ollama health check failed {}/{}: {e}",
                    health.consecutive_failures(),
                    config.ollama.max_health_failures
                );
                status.record_ollama_health(
                    false,
                    Some(format!(
                        "health check failed {}/{}: {e}",
                        health.consecutive_failures(),
                        config.ollama.max_health_failures
                    )),
                );

                if should_restart {
                    log::error!("Ollama unresponsive after {} consecutive failures", health.consecutive_failures());
                    let now = Instant::now();
                    if !limiter.try_record_restart(now) {
                        let reason = format!(
                            "raggiunto il limite di {} restart in {}s",
                            config.restart.max_restarts_per_window,
                            config.restart.restart_window_seconds
                        );
                        log::error!("FAULT: {reason}");
                        status.record_restarts_in_window(limiter.restarts_in_window(now) as u32);
                        status.enter_fault(reason.clone());
                        state = GuardState::Fault(reason);
                        continue;
                    }
                    status.record_restarts_in_window(limiter.restarts_in_window(now) as u32);

                    match force_restart_ollama(&config).await {
                        Ok(()) => {
                            health.record_success();
                            status.record_ollama_health(true, None);
                        }
                        Err(e) => {
                            log::error!("forced restart failed: {e}");
                            status.record_ollama_health(false, Some(e));
                        }
                    }
                } else {
                    status.record_restarts_in_window(limiter.restarts_in_window(Instant::now()) as u32);
                }
            }
        }
    }
}

/// Loop di monitoraggio GPU indipendente dal loop di health-check HTTP.
pub async fn run_gpu_watchdog(config: Config, status: SharedStatus) {
    let mut guard = GpuThermalGuard::new();
    let interval = Duration::from_secs(config.gpu.gpu_check_interval_seconds);

    loop {
        tokio::time::sleep(interval).await;

        let stats = match read_gpu_stats() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("could not read GPU stats: {e}");
                continue;
            }
        };
        status.record_gpu_temperature(stats.temperature_c);

        let action = guard.evaluate(
            stats.temperature_c,
            config.gpu.gpu_warning_temperature,
            config.gpu.gpu_critical_temperature,
            config.gpu.gpu_resume_temperature,
        );

        match action {
            GpuAction::Nothing => {}
            GpuAction::LogWarning => {
                log::warn!("GPU temperature {}C", stats.temperature_c);
            }
            GpuAction::StopCritical => {
                log::error!(
                    "CRITICAL GPU temperature {}C - stopping Ollama",
                    stats.temperature_c
                );
                status.enter_thermal_hold(format!(
                    "GPU temperature {}C >= critical threshold {}C",
                    stats.temperature_c, config.gpu.gpu_critical_temperature
                ));
                // Deleghiamo l'intera escalation SIGTERM→SIGKILL a systemd
                // (`TimeoutStopSec`/`SendSIGKILL=yes`/`KillMode=control-group`
                // configurati sull'unit): qui ci limitiamo a chiedere lo
                // stop e a VERIFICARE con polling reale (non fidandoci del
                // tempo di ritorno della CLI) che il processo sia
                // effettivamente sparito entro una deadline bounded.
                if let Err(e) = process::stop_ollama().await {
                    log::warn!("systemctl stop reported an error (will still verify real process state): {e}");
                }

                let confirm_timeout = Duration::from_secs(config.restart.recovery_timeout_seconds);
                let poll_interval = Duration::from_millis(300);
                let deadline = tokio::time::Instant::now() + confirm_timeout;
                let mut confirmed_stopped = false;
                loop {
                    match process::any_ollama_process_running().await {
                        Ok(false) => {
                            confirmed_stopped = true;
                            break;
                        }
                        Ok(true) => {}
                        Err(e) => log::warn!("could not verify Ollama process liveness: {e}"),
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(poll_interval).await;
                }

                if confirmed_stopped {
                    log::error!(
                        "Ollama stopped due to critical GPU temperature; will stay stopped until temperature drops below {}C",
                        config.gpu.gpu_resume_temperature
                    );
                } else {
                    let msg = format!(
                        "CRITICAL: Ollama process still present {}s after thermal stop was requested (systemd's own TimeoutStopSec/SendSIGKILL should have forced it down); manual operator intervention may be required",
                        confirm_timeout.as_secs()
                    );
                    log::error!("{msg}");
                    status.enter_thermal_hold(msg);
                }
            }
            GpuAction::StillTooHotToResume => {
                log::warn!(
                    "GPU still above resume threshold ({}C < {}C required), keeping Ollama stopped",
                    config.gpu.gpu_resume_temperature,
                    stats.temperature_c
                );
                status.enter_thermal_hold(format!(
                    "GPU still at {}C, below required resume threshold {}C not yet reached",
                    stats.temperature_c, config.gpu.gpu_resume_temperature
                ));
            }
            GpuAction::ResumeAllowed => {
                log::info!(
                    "GPU temperature back to {}C, below resume threshold {}C: restart allowed again",
                    stats.temperature_c,
                    config.gpu.gpu_resume_temperature
                );
                status.resume_from_thermal_hold();
                // Non riavviamo automaticamente qui: lasciamo che sia
                // l'operatore o il loop di health-check a farlo, secondo i
                // requisiti ("NON riavviare Ollama automaticamente").
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_monitor_triggers_restart_after_threshold() {
        let mut monitor = HealthMonitor::new(3);
        assert!(!monitor.record_failure()); // 1
        assert!(!monitor.record_failure()); // 2
        assert!(monitor.record_failure()); // 3 -> trigger
        assert_eq!(monitor.consecutive_failures(), 3);
    }

    #[test]
    fn health_monitor_resets_on_success() {
        let mut monitor = HealthMonitor::new(3);
        monitor.record_failure();
        monitor.record_failure();
        monitor.record_success();
        assert_eq!(monitor.consecutive_failures(), 0);
        assert!(!monitor.record_failure());
    }

    #[test]
    fn restart_limiter_allows_up_to_the_limit() {
        let mut limiter = RestartLimiter::new(Duration::from_secs(600), 3);
        let base = Instant::now();
        assert!(limiter.try_record_restart(base));
        assert!(limiter.try_record_restart(base + Duration::from_secs(10)));
        assert!(limiter.try_record_restart(base + Duration::from_secs(20)));
        // Il quarto restart nella stessa finestra deve essere rifiutato:
        // qui il chiamante deve entrare in stato FAULT.
        assert!(!limiter.try_record_restart(base + Duration::from_secs(30)));
    }

    #[test]
    fn restart_limiter_forgets_old_restarts_outside_window() {
        let mut limiter = RestartLimiter::new(Duration::from_secs(600), 3);
        let base = Instant::now();
        assert!(limiter.try_record_restart(base));
        assert!(limiter.try_record_restart(base + Duration::from_secs(10)));
        assert!(limiter.try_record_restart(base + Duration::from_secs(20)));
        // Dopo che la finestra è scaduta per i primi eventi, deve tornare
        // consentito.
        let much_later = base + Duration::from_secs(700);
        assert_eq!(limiter.restarts_in_window(much_later), 0);
        assert!(limiter.try_record_restart(much_later));
    }

    #[test]
    fn gpu_guard_warns_then_stops_on_critical() {
        let mut guard = GpuThermalGuard::new();
        assert_eq!(guard.evaluate(70, 80, 88, 75), GpuAction::Nothing);
        assert_eq!(guard.evaluate(82, 80, 88, 75), GpuAction::LogWarning);
        assert_eq!(guard.evaluate(90, 80, 88, 75), GpuAction::StopCritical);
        assert!(guard.is_holding());
    }

    #[test]
    fn gpu_guard_keeps_holding_until_resume_threshold() {
        let mut guard = GpuThermalGuard::new();
        guard.evaluate(90, 80, 88, 75); // trips critical
        assert_eq!(guard.evaluate(80, 80, 88, 75), GpuAction::StillTooHotToResume);
        assert!(guard.is_holding());
        assert_eq!(guard.evaluate(70, 80, 88, 75), GpuAction::ResumeAllowed);
        assert!(!guard.is_holding());
        // Una volta risolto, torna a comportarsi normalmente.
        assert_eq!(guard.evaluate(50, 80, 88, 75), GpuAction::Nothing);
    }
}
