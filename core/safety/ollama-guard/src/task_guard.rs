//! Task Guard: protezione contro un modello che, pur con Ollama
//! perfettamente in salute, entra in un loop di azioni ripetitive.
//!
//! Pensato per essere usato in futuro dall'orchestratore MCP: non ha
//! funzioni generiche per modificare il filesystem, si limita a
//! autorizzare/negare l'esecuzione di un'azione in base allo stato del
//! task. Un blocco qui termina SOLO il singolo task, non tocca Ollama.
//!
//! Il binario attuale non lo collega ancora a nessun loop (non esiste
//! ancora un orchestratore MCP da servire): l'intera API pubblica è quindi
//! "dead code" agli occhi del compilatore, silenziato qui volutamente.
#![allow(dead_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDecision {
    Allow,
    Blocked(String),
}

/// Configurazione dei limiti applicati a ogni task, tipicamente derivata da
/// `TaskGuardConfig` (vedi `config.rs`).
#[derive(Debug, Clone, Copy)]
pub struct TaskLimits {
    pub max_task_seconds: u64,
    pub max_actions_per_task: u32,
    pub max_errors: u32,
    pub max_identical_actions: u32,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub task_id: String,
    start_time: Instant,
    limits: TaskLimits,
    action_count: u32,
    error_count: u32,
    state: TaskState,
    last_action_signature: Option<String>,
    identical_run_count: u32,
}

impl Task {
    pub fn new(task_id: impl Into<String>, limits: TaskLimits, now: Instant) -> Self {
        Self {
            task_id: task_id.into(),
            start_time: now,
            limits,
            action_count: 0,
            error_count: 0,
            state: TaskState::Running,
            last_action_signature: None,
            identical_run_count: 0,
        }
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn action_count(&self) -> u32 {
        self.action_count
    }

    pub fn error_count(&self) -> u32 {
        self.error_count
    }

    /// Verifica se il task ha superato il tempo massimo consentito. Se sì,
    /// lo marca come `Killed`.
    pub fn check_timeout(&mut self, now: Instant) -> TaskDecision {
        if self.state != TaskState::Running {
            return TaskDecision::Blocked(format!("task già in stato {:?}", self.state));
        }
        let elapsed = now.duration_since(self.start_time);
        if elapsed >= Duration::from_secs(self.limits.max_task_seconds) {
            self.state = TaskState::Killed;
            return TaskDecision::Blocked(format!(
                "timeout: {}s >= limite {}s",
                elapsed.as_secs(),
                self.limits.max_task_seconds
            ));
        }
        TaskDecision::Allow
    }

    /// Registra la richiesta di eseguire un'azione (tool/comando) con una
    /// "firma" che identifica univocamente comando+argomenti. Ritorna se
    /// l'azione è autorizzata oppure il task va bloccato.
    ///
    /// Regole applicate, in ordine:
    /// 1. il task deve essere ancora `Running`;
    /// 2. non deve aver superato il timeout;
    /// 3. non deve aver superato il numero massimo di azioni;
    /// 4. la stessa azione non deve ripetersi identica troppe volte di
    ///    seguito (rilevamento loop).
    pub fn record_action(&mut self, signature: &str, now: Instant) -> TaskDecision {
        if self.state != TaskState::Running {
            return TaskDecision::Blocked(format!("task già in stato {:?}", self.state));
        }

        if let TaskDecision::Blocked(reason) = self.check_timeout(now) {
            return TaskDecision::Blocked(reason);
        }

        if self.action_count >= self.limits.max_actions_per_task {
            self.state = TaskState::Failed;
            return TaskDecision::Blocked(format!(
                "superato il numero massimo di azioni ({})",
                self.limits.max_actions_per_task
            ));
        }

        match &self.last_action_signature {
            Some(prev) if prev == signature => {
                self.identical_run_count += 1;
            }
            _ => {
                self.identical_run_count = 1;
                self.last_action_signature = Some(signature.to_string());
            }
        }

        if self.identical_run_count > self.limits.max_identical_actions {
            self.state = TaskState::Killed;
            return TaskDecision::Blocked(format!(
                "azione ripetuta identica {} volte consecutive (limite {}): possibile loop",
                self.identical_run_count, self.limits.max_identical_actions
            ));
        }

        self.action_count += 1;
        TaskDecision::Allow
    }

    /// Registra un errore restituito dall'esecuzione di un'azione. Se il
    /// numero massimo di errori viene superato, il task fallisce (ma Ollama
    /// non viene toccato: è un problema del singolo task/agente).
    pub fn record_error(&mut self) -> TaskDecision {
        if self.state != TaskState::Running {
            return TaskDecision::Blocked(format!("task già in stato {:?}", self.state));
        }
        self.error_count += 1;
        if self.error_count >= self.limits.max_errors {
            self.state = TaskState::Failed;
            return TaskDecision::Blocked(format!(
                "superato il numero massimo di errori ({})",
                self.limits.max_errors
            ));
        }
        TaskDecision::Allow
    }

    pub fn mark_completed(&mut self) {
        if self.state == TaskState::Running {
            self.state = TaskState::Completed;
        }
    }
}

/// Registro di tutti i task attivi/conclusi. Punto di ingresso pensato per
/// l'orchestratore MCP.
#[derive(Debug, Default)]
pub struct TaskGuard {
    tasks: HashMap<String, Task>,
}

impl TaskGuard {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn register_task(&mut self, task_id: impl Into<String>, limits: TaskLimits, now: Instant) {
        let task_id = task_id.into();
        self.tasks.insert(task_id.clone(), Task::new(task_id, limits, now));
    }

    pub fn get(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    /// Autorizza (o nega) l'esecuzione di un'azione per un task noto.
    /// Se il task non è registrato, l'azione è negata per default
    /// (fail closed).
    pub fn authorize_action(&mut self, task_id: &str, signature: &str, now: Instant) -> TaskDecision {
        match self.tasks.get_mut(task_id) {
            Some(task) => task.record_action(signature, now),
            None => TaskDecision::Blocked(format!("task sconosciuto: {task_id}")),
        }
    }

    pub fn record_error(&mut self, task_id: &str) -> TaskDecision {
        match self.tasks.get_mut(task_id) {
            Some(task) => task.record_error(),
            None => TaskDecision::Blocked(format!("task sconosciuto: {task_id}")),
        }
    }

    pub fn check_timeout(&mut self, task_id: &str, now: Instant) -> TaskDecision {
        match self.tasks.get_mut(task_id) {
            Some(task) => task.check_timeout(now),
            None => TaskDecision::Blocked(format!("task sconosciuto: {task_id}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> TaskLimits {
        TaskLimits {
            max_task_seconds: 300,
            max_actions_per_task: 50,
            max_errors: 5,
            max_identical_actions: 3,
        }
    }

    #[test]
    fn timeout_kills_task() {
        let now = Instant::now();
        let mut task = Task::new("t1", limits(), now);
        let later = now + Duration::from_secs(301);
        let decision = task.check_timeout(later);
        assert!(matches!(decision, TaskDecision::Blocked(_)));
        assert_eq!(task.state(), TaskState::Killed);
    }

    #[test]
    fn timeout_does_not_trigger_before_limit() {
        let now = Instant::now();
        let mut task = Task::new("t1", limits(), now);
        let soon = now + Duration::from_secs(299);
        assert_eq!(task.check_timeout(soon), TaskDecision::Allow);
        assert_eq!(task.state(), TaskState::Running);
    }

    #[test]
    fn max_actions_blocks_task() {
        let now = Instant::now();
        let mut task = Task::new(
            "t1",
            TaskLimits {
                max_task_seconds: 300,
                max_actions_per_task: 2,
                max_errors: 5,
                max_identical_actions: 100, // non è il fattore limitante qui
            },
            now,
        );
        assert_eq!(task.record_action("cmd_a", now), TaskDecision::Allow);
        assert_eq!(task.record_action("cmd_b", now), TaskDecision::Allow);
        let decision = task.record_action("cmd_c", now);
        assert!(matches!(decision, TaskDecision::Blocked(_)));
        assert_eq!(task.state(), TaskState::Failed);
    }

    #[test]
    fn identical_repeated_actions_are_detected_as_loop() {
        let now = Instant::now();
        let mut task = Task::new("t1", limits(), now);
        // create_directory(cliente1) ripetuto: le prime `max_identical_actions`
        // sono consentite, oltre viene bloccato.
        assert_eq!(
            task.record_action("create_directory(cliente1)", now),
            TaskDecision::Allow
        );
        assert_eq!(
            task.record_action("create_directory(cliente1)", now),
            TaskDecision::Allow
        );
        assert_eq!(
            task.record_action("create_directory(cliente1)", now),
            TaskDecision::Allow
        );
        let decision = task.record_action("create_directory(cliente1)", now);
        assert!(matches!(decision, TaskDecision::Blocked(_)));
        assert_eq!(task.state(), TaskState::Killed);
    }

    #[test]
    fn different_actions_reset_the_identical_counter() {
        let now = Instant::now();
        let mut task = Task::new("t1", limits(), now);
        assert_eq!(task.record_action("a", now), TaskDecision::Allow);
        assert_eq!(task.record_action("a", now), TaskDecision::Allow);
        assert_eq!(task.record_action("b", now), TaskDecision::Allow);
        assert_eq!(task.record_action("a", now), TaskDecision::Allow);
        assert_eq!(task.state(), TaskState::Running);
    }

    #[test]
    fn blocked_task_stays_blocked() {
        let now = Instant::now();
        let mut task = Task::new("t1", limits(), now);
        task.check_timeout(now + Duration::from_secs(400));
        assert_eq!(task.state(), TaskState::Killed);
        let decision = task.record_action("anything", now + Duration::from_secs(401));
        assert!(matches!(decision, TaskDecision::Blocked(_)));
    }

    #[test]
    fn task_guard_registers_and_authorizes() {
        let now = Instant::now();
        let mut guard = TaskGuard::new();
        guard.register_task("t1", limits(), now);
        assert_eq!(
            guard.authorize_action("t1", "list_dir(/tmp)", now),
            TaskDecision::Allow
        );
    }

    #[test]
    fn task_guard_denies_unknown_task_by_default() {
        let now = Instant::now();
        let mut guard = TaskGuard::new();
        let decision = guard.authorize_action("ghost", "anything", now);
        assert!(matches!(decision, TaskDecision::Blocked(_)));
    }

    #[test]
    fn errors_over_limit_fail_the_task_without_touching_ollama() {
        let mut guard = TaskGuard::new();
        let now = Instant::now();
        guard.register_task(
            "t1",
            TaskLimits {
                max_task_seconds: 300,
                max_actions_per_task: 50,
                max_errors: 2,
                max_identical_actions: 3,
            },
            now,
        );
        assert_eq!(guard.record_error("t1"), TaskDecision::Allow);
        let decision = guard.record_error("t1");
        assert!(matches!(decision, TaskDecision::Blocked(_)));
        assert_eq!(guard.get("t1").unwrap().state(), TaskState::Failed);
    }
}
