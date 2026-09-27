//! Pubblicazione dello stato di ollama-guard su un file JSON, in sola
//! lettura per chiunque altro (in particolare per il server MCP `mcp-ale`,
//! che gira come utente non privilegiato e legge questo file senza mai
//! usare sudo).
//!
//! ARCHITETTURA:
//! ```text
//! ollama-guard (root)
//!     |  scrive stato atomico
//!     v
//! /run/ollama-guard/status.json
//!     |  sola lettura
//!     v
//! mcp-ale (utente non privilegiato)
//!     |
//!     v
//! tool MCP ollama_guard_status
//! ```
//!
//! Il file NON deve mai contenere segreti: solo stato osservabile
//! (salute di Ollama, temperatura GPU, contatori, stato del watchdog).
//!
//! Scrittura ATOMICA: si scrive prima su un file temporaneo nella stessa
//! directory e poi si fa `rename` sul percorso finale, cosà che un lettore
//! concorrente non possa mai vedere un JSON troncato/parziale (il rename è
//! atomico sullo stesso filesystem).
//!
//! Permessi: il file viene creato con modalità `0644` (rw per il
//! proprietario, sola lettura per chiunque altro). Dato che ollama-guard
//! gira come `root` e mcp-ale come un utente diverso e non privilegiato,
//! questo è sufficiente perché mcp-ale possa leggere ma non possa mai
//! scrivere il file (nessun bit di scrittura per "altri").

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Stato complessivo riportato nel file, coerente con gli stati ammessi
/// dal watchdog: `WATCHING` (normale), `THERMAL_HOLD` (fermato per
/// temperatura critica, in attesa di rientro sotto soglia), `FAULT`
/// (limite di restart superato, richiede intervento umano).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Watching,
    ThermalHold,
    Fault,
}

impl StatusState {
    fn as_str(&self) -> &'static str {
        match self {
            StatusState::Watching => "WATCHING",
            StatusState::ThermalHold => "THERMAL_HOLD",
            StatusState::Fault => "FAULT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OllamaHealth {
    Unknown,
    Healthy,
    Unhealthy,
}

impl OllamaHealth {
    fn as_str(&self) -> &'static str {
        match self {
            OllamaHealth::Unknown => "unknown",
            OllamaHealth::Healthy => "healthy",
            OllamaHealth::Unhealthy => "unhealthy",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct StatusReport {
    state: &'static str,
    ollama: &'static str,
    gpu_temperature_c: Option<u32>,
    restarts_10m: u32,
    last_error: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct Inner {
    state: StatusState,
    ollama: OllamaHealth,
    gpu_temperature_c: Option<u32>,
    restarts_10m: u32,
    last_error: Option<String>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            state: StatusState::Watching,
            ollama: OllamaHealth::Unknown,
            gpu_temperature_c: None,
            restarts_10m: 0,
            last_error: None,
        }
    }
}

/// Handle condivisibile tra i due loop del watchdog (health e GPU) per
/// pubblicare uno stato coerente su file, con scrittura atomica.
#[derive(Clone)]
pub struct SharedStatus {
    inner: Arc<Mutex<Inner>>,
    path: Arc<PathBuf>,
}

impl SharedStatus {
    /// Crea l'handle e scrive subito uno stato iniziale, così il file
    /// esiste fin dall'avvio del servizio e non solo dopo il primo tick di
    /// uno dei due loop.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        let shared = Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            path: Arc::new(path.into()),
        };
        shared.publish();
        shared
    }

    /// Aggiorna lo stato con l'esito dell'ultimo health check verso
    /// Ollama. Da chiamare solo quando il check è stato realmente
    /// eseguito (il chiamante salta questa chiamata quando il watchdog è
    /// già in FAULT/THERMAL_HOLD e non interviene).
    pub fn record_ollama_health(&self, healthy: bool, error: Option<String>) {
        let mut inner = self.lock();
        inner.ollama = if healthy {
            OllamaHealth::Healthy
        } else {
            OllamaHealth::Unhealthy
        };
        if healthy {
            // Puliamo l'errore solo se non siamo in uno stato di guardia
            // attivo: un FAULT/THERMAL_HOLD deve restare visibile anche
            // se, per assurdo, un check isolato tornasse sano.
            if inner.state == StatusState::Watching {
                inner.last_error = None;
            }
        } else if let Some(e) = error {
            inner.last_error = Some(e);
        }
        self.publish_locked(&inner);
    }

    /// Aggiorna la temperatura GPU più recente letta da `nvidia-smi`.
    pub fn record_gpu_temperature(&self, temperature_c: u32) {
        let mut inner = self.lock();
        inner.gpu_temperature_c = Some(temperature_c);
        self.publish_locked(&inner);
    }

    /// Aggiorna il conteggio di restart nella finestra scorrevole (di
    /// norma 10 minuti, secondo `restart_window_seconds` in config).
    pub fn record_restarts_in_window(&self, count: u32) {
        let mut inner = self.lock();
        inner.restarts_10m = count;
        self.publish_locked(&inner);
    }

    /// Il watchdog ha superato il limite di restart consentiti: entra in
    /// FAULT. Questo stato è "sticky": nessun'altra transizione (thermal
    /// hold, salute tornata ok) può rimuoverlo; serve un riavvio del
    /// processo ollama-guard (dopo intervento umano) per uscirne.
    pub fn enter_fault(&self, reason: impl Into<String>) {
        let mut inner = self.lock();
        inner.state = StatusState::Fault;
        inner.last_error = Some(reason.into());
        self.publish_locked(&inner);
    }

    /// La GPU ha superato la soglia critica: Ollama è stato fermato e
    /// resterà fermo finché la temperatura non rientra. Non sovrascrive
    /// un FAULT già attivo (che ha priorità ed è definitivo).
    pub fn enter_thermal_hold(&self, reason: impl Into<String>) {
        let mut inner = self.lock();
        if inner.state != StatusState::Fault {
            inner.state = StatusState::ThermalHold;
        }
        inner.last_error = Some(reason.into());
        self.publish_locked(&inner);
    }

    /// La temperatura GPU è rientrata sotto la soglia di ripristino: si
    /// torna a WATCHING, ma solo se lo stato corrente era davvero
    /// THERMAL_HOLD (non deve mai "resuscitare" da un FAULT).
    pub fn resume_from_thermal_hold(&self) {
        let mut inner = self.lock();
        if inner.state == StatusState::ThermalHold {
            inner.state = StatusState::Watching;
            inner.last_error = None;
        }
        self.publish_locked(&inner);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn publish(&self) {
        let inner = self.lock();
        self.publish_locked(&inner);
    }

    fn publish_locked(&self, inner: &Inner) {
        let report = StatusReport {
            state: inner.state.as_str(),
            ollama: inner.ollama.as_str(),
            gpu_temperature_c: inner.gpu_temperature_c,
            restarts_10m: inner.restarts_10m,
            last_error: inner.last_error.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = write_atomic(&self.path, &report) {
            log::warn!("could not write status file {}: {e}", self.path.display());
        }
    }
}

fn write_atomic(path: &Path, report: &StatusReport) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "status.json".to_string());
    let tmp_path = dir.join(format!(".{file_name}.tmp"));

    let json = serde_json::to_string_pretty(report)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        // Sola lettura per chiunque non sia il proprietario (root in
        // produzione): mcp-ale deve poter leggere ma mai scrivere.
        f.set_permissions(fs::Permissions::from_mode(0o644))?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_status_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ollama-guard-status-test-{name}-{}", std::process::id()));
        p
    }

    #[test]
    fn writes_initial_watching_state() {
        let path = temp_status_path("initial");
        let _shared = SharedStatus::new(&path);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"state\": \"WATCHING\""));
        assert!(raw.contains("\"ollama\": \"unknown\""));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn records_health_and_clears_error_when_watching() {
        let path = temp_status_path("health");
        let shared = SharedStatus::new(&path);
        shared.record_ollama_health(false, Some("timeout".to_string()));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"ollama\": \"unhealthy\""));
        assert!(raw.contains("timeout"));

        shared.record_ollama_health(true, None);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"ollama\": \"healthy\""));
        assert!(raw.contains("\"last_error\": null"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn fault_state_is_sticky_and_not_overridden_by_thermal_or_health() {
        let path = temp_status_path("fault");
        let shared = SharedStatus::new(&path);
        shared.enter_fault("troppi restart");
        shared.enter_thermal_hold("critica");
        shared.record_ollama_health(true, None);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"state\": \"FAULT\""));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn thermal_hold_resumes_to_watching_only_from_thermal_hold() {
        let path = temp_status_path("thermal");
        let shared = SharedStatus::new(&path);
        shared.enter_thermal_hold("temperatura critica");
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"state\": \"THERMAL_HOLD\""));

        shared.resume_from_thermal_hold();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"state\": \"WATCHING\""));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn restart_counter_is_reported() {
        let path = temp_status_path("restarts");
        let shared = SharedStatus::new(&path);
        shared.record_restarts_in_window(2);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"restarts_10m\": 2"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn write_is_atomic_no_temp_file_left_behind() {
        let path = temp_status_path("atomic");
        let shared = SharedStatus::new(&path);
        shared.record_gpu_temperature(42);
        let dir = path.parent().unwrap();
        let tmp_name = format!(
            ".{}.tmp",
            path.file_name().unwrap().to_string_lossy()
        );
        assert!(!dir.join(&tmp_name).exists());
        fs::remove_file(&path).ok();
    }
}
