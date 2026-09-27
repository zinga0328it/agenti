//! Harness locale temporaneo per testare il Task Guard senza un vero
//! orchestratore MCP (non ancora esistente). Simula esattamente lo
//! scenario dei requisiti: un "agente" che chiama ripetutamente
//! `create_directory("cliente1")` con gli stessi argomenti, oltre il
//! limite `max_identical_actions`, e verifica che il Task Guard blocchi il
//! task SENZA toccare Ollama.
//!
//! Esegui con: `cargo run --example task_guard_harness`
use ollama_guard::task_guard::{TaskDecision, TaskGuard, TaskLimits};
use std::time::{Duration, Instant};

/// Simula l'esecuzione di un tool MCP: chiede autorizzazione al Task
/// Guard PRIMA di "eseguire" l'azione. Se negata, l'azione non viene
/// eseguita (qui semplicemente stampiamo cosa sarebbe successo).
fn try_call_tool(guard: &mut TaskGuard, task_id: &str, tool_call: &str, now: Instant) -> bool {
    match guard.authorize_action(task_id, tool_call, now) {
        TaskDecision::Allow => {
            println!("  [ALLOW]   {tool_call} eseguito");
            true
        }
        TaskDecision::Blocked(reason) => {
            println!("  [BLOCKED] {tool_call} NEGATO: {reason}");
            false
        }
    }
}

fn separator(title: &str) {
    println!("\n=== {title} ===");
}

fn main() {
    // --- Scenario 1: azioni identiche ripetute (create_directory in loop) ---
    separator("Scenario 1: create_directory(\"cliente1\") ripetuto in loop");
    let limits_loop = TaskLimits {
        max_task_seconds: 300,
        max_actions_per_task: 50,
        max_errors: 5,
        max_identical_actions: 3,
    };
    let mut guard = TaskGuard::new();
    let now = Instant::now();
    guard.register_task("task-loop-1", limits_loop, now);

    let calls = [
        "create_directory(cliente1)",
        "create_directory(cliente1)",
        "create_directory(cliente1)",
        "create_directory(cliente1)", // questa deve essere negata (4a identica consecutiva)
    ];
    let mut allowed_count = 0;
    for call in calls {
        if try_call_tool(&mut guard, "task-loop-1", call, now) {
            allowed_count += 1;
        }
    }
    let final_state = guard.get("task-loop-1").unwrap().state();
    println!("Azioni autorizzate: {allowed_count}/4 (attese: 3/4)");
    println!("Stato finale del task: {final_state:?} (atteso: Killed)");
    assert_eq!(allowed_count, 3, "solo le prime 3 azioni identiche devono passare");
    assert_eq!(
        final_state,
        ollama_guard::task_guard::TaskState::Killed,
        "il task deve risultare KILLED dopo il loop"
    );
    println!("Ollama NON è stato toccato: il Task Guard agisce solo sul singolo task.");

    // --- Scenario 2: limite massimo di azioni per task ---
    separator("Scenario 2: max_actions_per_task");
    let limits_actions = TaskLimits {
        max_task_seconds: 300,
        max_actions_per_task: 5,
        max_errors: 5,
        max_identical_actions: 100, // alto apposta, per isolare il limite di azioni
    };
    let mut guard2 = TaskGuard::new();
    guard2.register_task("task-actions", limits_actions, now);
    let mut allowed = 0;
    for i in 0..8 {
        let call = format!("list_files(dir{i})"); // ogni azione è diversa
        if try_call_tool(&mut guard2, "task-actions", &call, now) {
            allowed += 1;
        }
    }
    println!("Azioni autorizzate: {allowed}/8 (attese: 5/8, limite max_actions_per_task=5)");
    assert_eq!(allowed, 5);
    let state2 = guard2.get("task-actions").unwrap().state();
    println!("Stato finale: {state2:?} (atteso: Failed, distinto da Killed usato per loop/timeout)");
    assert_eq!(state2, ollama_guard::task_guard::TaskState::Failed);

    // --- Scenario 3: timeout massimo del task ---
    separator("Scenario 3: max_task_seconds");
    let limits_timeout = TaskLimits {
        max_task_seconds: 10,
        max_actions_per_task: 1000,
        max_errors: 5,
        max_identical_actions: 1000,
    };
    let mut guard3 = TaskGuard::new();
    let start = Instant::now();
    guard3.register_task("task-timeout", limits_timeout, start);

    let before_timeout = start + Duration::from_secs(5);
    let decision_early = guard3.check_timeout("task-timeout", before_timeout);
    println!(
        "A 5s (< 10s limite): {decision_early:?} (atteso: Allow, il task deve poter proseguire)"
    );
    assert_eq!(decision_early, TaskDecision::Allow);

    let after_timeout = start + Duration::from_secs(11);
    let decision_late = guard3.check_timeout("task-timeout", after_timeout);
    println!("A 11s (> 10s limite): {decision_late:?} (atteso: Blocked)");
    assert!(matches!(decision_late, TaskDecision::Blocked(_)));
    println!(
        "Stato finale: {:?} (atteso: Killed)",
        guard3.get("task-timeout").unwrap().state()
    );

    println!("\nTutti gli scenari del Task Guard harness sono conformi alle attese.");
}
