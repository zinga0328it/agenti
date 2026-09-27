//! Health check di Ollama, completamente indipendente dalla logica interna
//! di Ollama stesso: usiamo solo una richiesta HTTP con timeout stretto
//! verso un endpoint leggero.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("richiesta HTTP fallita: {0}")]
    Request(#[from] reqwest::Error),
    #[error("risposta HTTP non valida: status {0}")]
    BadStatus(reqwest::StatusCode),
}

/// Esegue un singolo controllo di salute su Ollama chiamando `/api/tags`,
/// un endpoint leggero che non richiede caricamento di modelli.
pub async fn check_health(base_url: &str, timeout: Duration) -> Result<(), HealthError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(HealthError::Request)?;

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = client.get(url).send().await?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(HealthError::BadStatus(response.status()))
    }
}

/// Attende che la porta di Ollama torni ad accettare connessioni TCP, con un
/// timeout complessivo. Usato dopo un riavvio per sapere quando il servizio
/// è di nuovo pronto, senza fidarsi ciecamente di systemd.
pub async fn wait_until_reachable(
    base_url: &str,
    poll_interval: Duration,
    overall_timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + overall_timeout;
    loop {
        if check_health(base_url, poll_interval).await.is_ok() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_health_fails_fast_when_nothing_listens() {
        // Porta improbabile: nessun servizio in ascolto durante i test.
        let result = check_health("http://127.0.0.1:1", Duration::from_millis(200)).await;
        assert!(result.is_err());
    }
}
