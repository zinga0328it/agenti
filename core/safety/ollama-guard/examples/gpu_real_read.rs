//! Verifica una tantum che il parser GPU legga valori REALI da
//! `nvidia-smi` (nessuno stress termico volontario: leggiamo solo lo stato
//! attuale della macchina, come richiesto per FASE 6 del test end-to-end).
//!
//! Esegui con: `cargo run --example gpu_real_read`
use ollama_guard::gpu::{classify_temperature, read_gpu_stats, ThermalLevel};

fn main() {
    match read_gpu_stats() {
        Ok(stats) => {
            println!("Lettura GPU reale riuscita (nvidia-smi):");
            println!("  temperatura:  {} C", stats.temperature_c);
            println!("  utilizzo:     {} %", stats.utilization_pct);
            println!(
                "  VRAM:         {} / {} MiB",
                stats.memory_used_mib, stats.memory_total_mib
            );
            println!("  power draw:   {} W", stats.power_draw_w);

            // Le soglie usate qui sono solo per dimostrare la classificazione
            // con dati reali odierni (tipicamente Normal), NON per simulare
            // WARNING/CRITICAL: quello si fa nei unit test con dati
            // sintetici (vedi src/gpu.rs #[cfg(test)]), non stressando la
            // GPU reale.
            let level = classify_temperature(stats.temperature_c, 80, 88);
            println!("  classificazione con soglie di esempio (warning=80, critical=88): {level:?}");
            match level {
                ThermalLevel::Normal => println!("  -> GPU in range normale, nessuna azione richiesta."),
                ThermalLevel::Warning => println!("  -> sopra WARNING (inatteso senza stress termico)."),
                ThermalLevel::Critical => println!("  -> sopra CRITICAL (inatteso senza stress termico)."),
            }

            assert!(stats.temperature_c > 0, "temperatura deve essere un valore reale positivo");
            assert!(stats.memory_total_mib > 0, "memoria totale deve essere un valore reale positivo");
            println!("\nParser GPU confermato funzionante su dati reali.");
        }
        Err(e) => {
            eprintln!("Errore lettura GPU reale: {e}");
            std::process::exit(1);
        }
    }
}
