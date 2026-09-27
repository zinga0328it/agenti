//! ollama-guard: watchdog indipendente per Ollama.
//!
//! Responsabilità:
//! - controllare periodicamente che Ollama risponda su HTTP (localhost);
//! - forzare stop/kill/restart in caso di blocco reale, con limite di
//!   restart per finestra temporale (stato FAULT oltre il limite);
//! - monitorare la temperatura GPU e fermare Ollama in caso di soglia
//!   critica, senza riavviarlo finché non rientra sotto soglia sicura;
//! - esporre un Task Guard (modulo a parte) che l'orchestratore MCP potrà
//!   usare per bloccare singoli task in loop, senza toccare Ollama.
//!
//! Questo binario NON modifica ancora systemd: va lanciato manualmente o in
//! foreground per test; l'unit systemd sarà aggiunta in un passo successivo.

mod config;
mod ollama;
mod process;
mod status;
mod watchdog;

// `task_guard` e `gpu` sono esposti anche come libreria (vedi `src/lib.rs`)
// per poter essere usati da `examples/` e da un futuro orchestratore MCP;
// qui nel binario li riutilizziamo tramite quella stessa libreria invece
// di ridichiarare i moduli, per evitare di compilarli due volte con
// percorsi diversi.
use ollama_guard::{gpu, task_guard};

use config::Config;
use status::SharedStatus;

fn config_path() -> String {
    std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ollama-guard.toml".to_string())
}

/// Percorso del file di stato pubblicato in sola lettura per il resto del
/// sistema (in particolare per il tool MCP `ollama_guard_status`).
///
/// Il default `/run/ollama-guard/status.json` presume `RuntimeDirectory=
/// ollama-guard` nell'unit systemd (systemd crea la directory con i
/// permessi giusti prima di eseguire il binario). L'override via env var
/// serve solo per i test manuali/di sviluppo eseguiti senza systemd e
/// senza privilegi su `/run`.
fn status_path() -> String {
    std::env::var("OLLAMA_GUARD_STATUS_PATH")
        .unwrap_or_else(|_| "/run/ollama-guard/status.json".to_string())
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let path = config_path();
    let config = match Config::load_from_file(&path) {
        Ok(c) => c,
        Err(e) => {
            // Fail closed: senza configurazione valida non avviamo nulla,
            // niente valori di default pericolosi.
            log::error!("cannot start: invalid configuration ({path}): {e}");
            std::process::exit(1);
        }
    };

    log::info!("ollama-guard starting, watching {}", config.ollama.url);

    let status_path = status_path();
    log::info!("publishing status to {status_path}");
    let shared_status = SharedStatus::new(status_path);

    let health_config = config.clone();
    let gpu_config = config.clone();
    let health_status = shared_status.clone();
    let gpu_status = shared_status;

    let health_task = tokio::spawn(watchdog::run_health_watchdog(health_config, health_status));
    let gpu_task = tokio::spawn(watchdog::run_gpu_watchdog(gpu_config, gpu_status));

    let _ = tokio::join!(health_task, gpu_task);
}
