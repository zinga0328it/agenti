//! Parte del progetto esposta come libreria.
//!
//! `task_guard` è pensato esplicitamente per essere riutilizzato da un
//! futuro orchestratore MCP (o, per ora, da un piccolo harness di test
//! locale in `examples/`). `gpu` è di sola lettura (nessuna azione
//! amministrativa) e viene esposto solo per poter verificare con un
//! piccolo esempio che il parser legga valori reali da `nvidia-smi` senza
//! dover duplicare la logica di parsing. Il resto del watchdog (health
//! check, process, config, watchdog) resta privato al binario perché non
//! ha bisogno di essere una libreria riutilizzabile.
pub mod gpu;
pub mod task_guard;
