//! Configurazione di ollama-guard.
//!
//! Tutte le soglie operative (temperature GPU, intervalli, limiti di
//! restart, limiti del Task Guard) sono lette da un file TOML esterno:
//! nessun valore critico deve restare hardcoded nel binario.

// TaskGuardConfig e alcuni suoi campi non sono ancora letti dal binario
// principale: il Task Guard è esposto come libreria per il futuro
// orchestratore MCP, non ancora collegato a un loop qui.
#![allow(dead_code)]

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaConfig {
    pub url: String,
    pub health_interval_seconds: u64,
    pub health_timeout_seconds: u64,
    pub max_health_failures: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestartConfig {
    pub restart_window_seconds: u64,
    pub max_restarts_per_window: u32,
    /// Tempo massimo concesso, DOPO aver chiesto a systemd un
    /// `restart`/`stop`, per verificare con polling reale (HTTP o presenza
    /// del processo) che l'azione abbia avuto davvero l'effetto atteso.
    ///
    /// ARCHITETTURA (revisione 2026-09-27): non reimplementiamo più
    /// l'escalation SIGTERM→SIGKILL in Rust: quella è delegata all'unit
    /// systemd (`TimeoutStopSec`, `KillMode=control-group`,
    /// `SendSIGKILL=yes`). Questo timeout serve solo per la nostra verifica
    /// indipendente del risultato, non per pilotare direttamente segnali.
    #[serde(default = "default_recovery_timeout_seconds")]
    pub recovery_timeout_seconds: u64,
}

fn default_recovery_timeout_seconds() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct GpuConfig {
    pub gpu_check_interval_seconds: u64,
    pub gpu_warning_temperature: u32,
    pub gpu_critical_temperature: u32,
    pub gpu_resume_temperature: u32,
}

fn default_max_errors_per_task() -> u32 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskGuardConfig {
    pub max_task_seconds: u64,
    pub max_actions_per_task: u32,
    pub max_identical_actions: u32,
    /// Non richiesto esplicitamente nella lista minima dei campi, ma
    /// necessario al Task Guard per il conteggio errori. Ha un default per
    /// restare compatibile con file di configurazione minimi.
    #[serde(default = "default_max_errors_per_task")]
    pub max_errors_per_task: u32,
}

impl From<&TaskGuardConfig> for crate::task_guard::TaskLimits {
    fn from(cfg: &TaskGuardConfig) -> Self {
        crate::task_guard::TaskLimits {
            max_task_seconds: cfg.max_task_seconds,
            max_actions_per_task: cfg.max_actions_per_task,
            max_errors: cfg.max_errors_per_task,
            max_identical_actions: cfg.max_identical_actions,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub ollama: OllamaConfig,
    pub restart: RestartConfig,
    pub gpu: GpuConfig,
    pub task_guard: TaskGuardConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("impossibile leggere il file di configurazione {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("file di configurazione non valido: {0}")]
    Parse(#[from] toml::de::Error),
}

impl Config {
    /// Carica la configurazione da un file TOML. Nessun default pericoloso
    /// viene applicato silenziosamente: se il file manca o è malformato
    /// l'errore va propagato e il programma deve rifiutarsi di partire
    /// (fail closed).
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let raw = std::fs::read_to_string(path_ref).map_err(|source| ConfigError::Read {
            path: path_ref.display().to_string(),
            source,
        })?;
        let config: Config = toml::from_str(&raw)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_config() {
        let raw = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/ollama-guard.toml"),
        )
        .expect("example config must exist");
        let config: Config = toml::from_str(&raw).expect("example config must parse");
        assert_eq!(config.ollama.max_health_failures, 3);
        assert_eq!(config.gpu.gpu_critical_temperature, 88);
        assert_eq!(config.task_guard.max_identical_actions, 3);
    }

    #[test]
    fn missing_file_is_an_error() {
        let result = Config::load_from_file("/nonexistent/ollama-guard.toml");
        assert!(result.is_err());
    }
}
