# systemd — MCP ALE

Copia documentata di `/etc/systemd/system/mcp-ale.service`.

## Note progettuali

- `User=mcp-ale` (utente di sistema dedicato, `--no-create-home`,
  `/usr/sbin/nologin`), non root: il server MCP è read-only e non ha
  bisogno di privilegi.
- `After=network-online.target yggdrasil.service nvidia-persistenced.service ollama.service`
  — parte solo dopo che la rete e Yggdrasil sono pronti; l'indirizzo IPv6
  reale di `ygg0` viene risolto a runtime da
  `scripts/mcp-ale/wait-for-ygg0.sh` (mai hardcoded nella unit).
- `Restart=always` / `RestartSec=3`.
- `NoNewPrivileges=true`, `PrivateTmp=true`. `ProtectSystem=strict` e
  `ProtectHome=true` sono stati rimossi deliberatamente: la working
  directory del servizio vive sotto `/home/ale`, quindi quelle protezioni
  causerebbero `203/EXEC` (stessa scelta già adottata da
  `ollama-guard.service` in una sessione precedente).
- `ExecStart` invoca `scripts/mcp-ale/start-mcp-ale.sh`, che risolve
  l'IPv6 di `ygg0`, esporta `MCP_HOST`/`MCP_ALLOWED_HOST` e infine esegue
  il server Python.

## Test reali eseguiti

- `systemd-analyze verify` → PASS.
- `systemctl enable` + `start` → servizio attivo, bind confermato SOLO su
  ygg0 (`ss -tlnp`).
- `systemctl restart mcp-ale.service` (simulazione di crash/riavvio) →
  il servizio è tornato attivo entro pochi secondi e un client MCP reale
  ha potuto riconnettersi e richiamare tutti e 3 i tool con successo.
