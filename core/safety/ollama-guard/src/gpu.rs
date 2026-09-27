//! Lettura dello stato della GPU tramite `nvidia-smi`, con soglie definite
//! interamente in configurazione (mai hardcoded).

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuStats {
    pub temperature_c: u32,
    pub utilization_pct: u32,
    pub memory_used_mib: u64,
    pub memory_total_mib: u64,
    pub power_draw_w: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("impossibile eseguire nvidia-smi: {0}")]
    Spawn(std::io::Error),
    #[error("nvidia-smi ha restituito uno stato di errore: {0}")]
    CommandFailed(String),
    #[error("output di nvidia-smi non parsabile: {0}")]
    Parse(String),
}

/// Interroga `nvidia-smi` per la prima GPU disponibile (indice 0) con una
/// query fissa in CSV, senza header e senza unità, per un parsing robusto.
pub fn read_gpu_stats() -> Result<GpuStats, GpuError> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total,power.draw",
            "--format=csv,noheader,nounits",
            "-i",
            "0",
        ])
        .output()
        .map_err(GpuError::Spawn)?;

    if !output.status.success() {
        return Err(GpuError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().ok_or_else(|| {
        GpuError::Parse("output vuoto".to_string())
    })?;
    parse_csv_line(line)
}

fn parse_csv_line(line: &str) -> Result<GpuStats, GpuError> {
    let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
    if fields.len() != 5 {
        return Err(GpuError::Parse(format!(
            "attesi 5 campi, trovati {} in '{}'",
            fields.len(),
            line
        )));
    }

    let parse_u32 = |s: &str| {
        s.parse::<u32>()
            .map_err(|_| GpuError::Parse(format!("valore intero non valido: '{}'", s)))
    };
    let parse_u64 = |s: &str| {
        s.parse::<u64>()
            .map_err(|_| GpuError::Parse(format!("valore intero non valido: '{}'", s)))
    };
    let parse_f64 = |s: &str| {
        s.parse::<f64>()
            .map_err(|_| GpuError::Parse(format!("valore decimale non valido: '{}'", s)))
    };

    Ok(GpuStats {
        temperature_c: parse_u32(fields[0])?,
        utilization_pct: parse_u32(fields[1])?,
        memory_used_mib: parse_u64(fields[2])?,
        memory_total_mib: parse_u64(fields[3])?,
        power_draw_w: parse_f64(fields[4])?,
    })
}

/// Stato derivato dal confronto tra temperatura corrente e soglie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLevel {
    Normal,
    Warning,
    Critical,
}

pub fn classify_temperature(
    temperature_c: u32,
    warning_threshold: u32,
    critical_threshold: u32,
) -> ThermalLevel {
    if temperature_c >= critical_threshold {
        ThermalLevel::Critical
    } else if temperature_c >= warning_threshold {
        ThermalLevel::Warning
    } else {
        ThermalLevel::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_csv_line() {
        let line = "82, 45, 4096, 12288, 123.45";
        let stats = parse_csv_line(line).unwrap();
        assert_eq!(stats.temperature_c, 82);
        assert_eq!(stats.utilization_pct, 45);
        assert_eq!(stats.memory_used_mib, 4096);
        assert_eq!(stats.memory_total_mib, 12288);
        assert!((stats.power_draw_w - 123.45).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_malformed_csv_line() {
        assert!(parse_csv_line("not,enough,fields").is_err());
    }

    #[test]
    fn classifies_thermal_levels() {
        assert_eq!(classify_temperature(70, 80, 88), ThermalLevel::Normal);
        assert_eq!(classify_temperature(80, 80, 88), ThermalLevel::Warning);
        assert_eq!(classify_temperature(85, 80, 88), ThermalLevel::Warning);
        assert_eq!(classify_temperature(88, 80, 88), ThermalLevel::Critical);
        assert_eq!(classify_temperature(95, 80, 88), ThermalLevel::Critical);
    }
}
