//! Interfaccia minimale e whitelisted verso systemd.
//!
//! Regole non negoziabili:
//! - nessun comando è costruito a partire da stringhe generate da un LLM o da
//!   input esterno: ogni funzione qui esegue un comando fisso con argomenti
//!   fissi, decisi a compile-time;
//! - nessuna shell arbitraria (`sh -c ...`): usiamo sempre `Command::new` con
//!   argomenti separati;
//! - in caso di dubbio o errore, le funzioni ritornano `Err` (fail closed) e
//!   non intraprendono azioni distruttive aggiuntive;
//! - in PRODUZIONE ollama-guard gira come servizio systemd con `User=root`:
//!   in quel caso NON deve mai usare `sudo`/`pkexec`/Polkit, chiama
//!   `systemctl` direttamente (root ha già l'autorità necessaria). Quando
//!   invece gira come utente normale (es. durante i test interattivi), le
//!   operazioni privilegiate passano per `sudo -n`: il flag `-n` fa fallire
//!   SUBITO la chiamata (niente prompt, niente finestre Polkit) se sudo
//!   richiedesse una password. La scelta tra le due modalità è automatica,
//!   basata sull'UID effettivo del processo, non su un flag di
//!   configurazione che potrebbe essere lasciato disallineato.
//!
//! ARCHITETTURA (revisione del 2026-09-27): questo modulo NON reimplementa
//! più la logica di escalation SIGTERM→SIGKILL. Quella responsabilità è
//! stata deliberatamente delegata a systemd, configurando `ollama.service`
//! con `TimeoutStopSec` breve, `KillMode=control-group` e `SendSIGKILL=yes`:
//! se il processo non risponde al SIGTERM entro il timeout, è systemd
//! stesso a forzare il SIGKILL sull'intero control-group. `ollama-guard` si
//! limita a chiedere l'azione (`restart`/`stop`) e poi VERIFICA il
//! risultato reale (salute HTTP o assenza di processi), senza mai fidarsi
//! ciecamente del fatto che la CLI amministrativa sia tornata in tempi
//! brevi.
//!
//! BUG STORICO (trovato nel test end-to-end reale con Ollama congelato via
//! SIGSTOP, prima di questa revisione): il codice avvolgeva le chiamate a
//! `systemctl` in un `tokio::time::timeout` con `kill_on_drop(true)`. Se il
//! timeout scadeva, il nostro stesso codice terminava il processo client
//! `systemctl` PRIMA che la sua richiesta D-Bus fosse anche solo consegnata
//! a PID 1, annullando di fatto il tentativo. Ora non serve più: dato che
//! l'unit gestisce da sola l'escalation con un timeout breve e
//! deterministico, ci limitiamo a lanciare il comando e a verificare
//! l'esito reale con polling indipendente.

use std::process::{ExitStatus, Output};
use std::sync::OnceLock;
use tokio::process::Command;

const OLLAMA_UNIT: &str = "ollama.service";

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("impossibile eseguire il comando `{0}`: {1}")]
    Spawn(&'static str, std::io::Error),
    #[error("comando `{0}` fallito con stato {1:?}: {2}")]
    Failed(&'static str, Option<i32>, String),
}

/// Determina se il processo corrente gira con UID effettivo 0 (root),
/// leggendo `/proc/self/status`. Cache statica: l'UID di un processo in
/// esecuzione non cambia, non serve rileggerlo a ogni chiamata.
fn running_as_root() -> bool {
    static IS_ROOT: OnceLock<bool> = OnceLock::new();
    *IS_ROOT.get_or_init(|| {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    let rest = line.strip_prefix("Uid:")?;
                    // Formato: "Uid:\t<real>\t<effective>\t<saved>\t<fs>"
                    let effective = rest.split_whitespace().nth(1)?;
                    effective.parse::<u32>().ok()
                })
            })
            .map(|effective_uid| effective_uid == 0)
            .unwrap_or(false) // fail closed: se non determinabile, assumiamo non-root
    })
}

async fn run(program: &'static str, args: &[&str]) -> Result<Output, ProcessError> {
    // kill_on_drop(false): se il chiamante smette di aspettare questo
    // future (es. dopo un timeout), il comando continua a girare in
    // background invece di essere ucciso a metà, così la sua richiesta
    // reale verso systemd/PID1 viene comunque consegnata. Vedi bug storico
    // documentato in cima al modulo.
    Command::new(program)
        .args(args)
        .kill_on_drop(false)
        .output()
        .await
        .map_err(|e| ProcessError::Spawn(program, e))
}

/// Esegue un comando amministrativo su `systemctl`, scegliendo
/// automaticamente se serve `sudo -n` in base all'UID effettivo del
/// processo (vedi `running_as_root`). Nessun testo esterno finisce negli
/// argomenti: solo `args` fissi, decisi a compile-time dal chiamante.
async fn run_systemctl_privileged(args: &[&str]) -> Result<Output, ProcessError> {
    if running_as_root() {
        run("systemctl", args).await
    } else {
        let mut full_args = Vec::with_capacity(args.len() + 2);
        full_args.push("-n");
        full_args.push("systemctl");
        full_args.extend_from_slice(args);
        Command::new("sudo")
            .args(&full_args)
            .kill_on_drop(false)
            .output()
            .await
            .map_err(|e| ProcessError::Spawn("sudo", e))
    }
}

fn ensure_success(program: &'static str, output: Output) -> Result<(), ProcessError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(ProcessError::Failed(
            program,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

/// Chiede a systemd di riavviare Ollama (stop + start atomici come singolo
/// job). Se il processo non risponde al SIGTERM, è l'unit stessa
/// (`TimeoutStopSec` + `SendSIGKILL=yes` + `KillMode=control-group`) a
/// forzare il SIGKILL entro un tempo bounded: questo modulo non reimplementa
/// quella logica. Il chiamante deve comunque verificare la salute reale di
/// Ollama dopo questa chiamata, non fidarsi del solo codice di uscita.
pub async fn restart_ollama() -> Result<(), ProcessError> {
    let output = run_systemctl_privileged(&["restart", OLLAMA_UNIT]).await?;
    ensure_success("systemctl restart", output)
}

/// Ferma Ollama (usato per il blocco termico GPU: qui NON vogliamo un
/// riavvio automatico). Uno stop esplicito via systemctl impedisce anche a
/// `Restart=on-failure` di rimettere in piedi il servizio da solo.
pub async fn stop_ollama() -> Result<(), ProcessError> {
    let output = run_systemctl_privileged(&["stop", OLLAMA_UNIT]).await?;
    ensure_success("systemctl stop", output)
}

/// Avvia Ollama. Non collegata automaticamente al rientro dal blocco
/// termico GPU (per requisito esplicito: il rientro da `ThermalHold` non
/// deve riavviare Ollama da solo), ma resta disponibile come azione
/// esplicita per un operatore o per un futuro comando amministrativo.
#[allow(dead_code)]
pub async fn start_ollama() -> Result<(), ProcessError> {
    let output = run_systemctl_privileged(&["start", OLLAMA_UNIT]).await?;
    ensure_success("systemctl start", output)
}

/// Verifica se il servizio risulta ancora attivo secondo systemd.
/// Ritorna `Ok(true)` se attivo, `Ok(false)` se non attivo, `Err` se lo stato
/// non è determinabile (fail closed: il chiamante deve trattarlo come
/// "potenzialmente ancora vivo"). Query di sola lettura: non richiede
/// privilegi, quindi niente `sudo`. Non ancora usata nel loop principale
/// (che si affida a `any_ollama_process_running`/verifica HTTP), ma utile
/// per diagnostica/tooling futuro.
#[allow(dead_code)]
pub async fn is_ollama_active() -> Result<bool, ProcessError> {
    let output = run("systemctl", &["is-active", "--quiet", OLLAMA_UNIT]).await?;
    Ok(output.status.success())
}

/// Verifica se esistono ancora processi del binario `ollama` in esecuzione,
/// indipendentemente dallo stato riportato da systemd. Query di sola
/// lettura: non richiede privilegi.
pub async fn any_ollama_process_running() -> Result<bool, ProcessError> {
    let output = run("pgrep", &["-x", "ollama"]).await?;
    // pgrep esce con 0 se trova processi, 1 se non ne trova, >1 su errore.
    match exit_code(output.status) {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ProcessError::Failed(
            "pgrep",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_as_root_reflects_real_uid() {
        // Il test gira quasi certamente come utente normale in CI/locale:
        // verifichiamo solo che la funzione non vada in panico e ritorni
        // un booleano coerente con l'UID reale del processo di test.
        let is_root = running_as_root();
        let real_uid_is_zero = std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u32>()
            .unwrap()
            == 0;
        assert_eq!(is_root, real_uid_is_zero);
    }
}
