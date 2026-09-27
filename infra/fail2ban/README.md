# Fail2Ban — MCP ALE

Copie documentate dei moduli Fail2Ban dedicati al servizio MCP. Le jail
pre-esistenti (`sshd`, `honeypot-scan`, `apache-badbots`, ecc.) **non sono
state toccate**, come richiesto: qui c'è solo il modulo nuovo, additivo.

## File

- `filter.d/mcp-ale.conf` → riconosce le righe reali di log del server MCP
  (`MCP-ALE-REJECT SRC=<host> STATUS=<code> PATH=<path>`), emesse dal
  middleware di sicurezza del server per ogni risposta HTTP ≥ 400
  (richieste non autorizzate, Host non valido/DNS-rebinding, richieste
  malformate). Il filtro NON è ancorato con `^` all'inizio riga, per lo
  stesso motivo del filtro pre-esistente `honeypot_scan.conf`: il prefisso
  `<host> <unit>[<pid>]:` aggiunto da journald/syslog varia a seconda del
  backend usato da Fail2Ban. Validato con `fail2ban-regex` sia su log
  estratto sia con `--journalmatch` diretto sul journal reale (8/8 righe
  reali riconosciute).
- `jail.d/mcp-ale.local` → jail `mcp-ale`, `backend = systemd`,
  `banaction = mcp-ale-nftables` (azione dedicata, vedi sotto).
- `mcp-ale-nftables.conf` → azione custom (va installata in
  `/etc/fail2ban/action.d/`). Necessaria perché l'azione di default
  `nftables-multiport` dipende da una tabella dinamica (`f2b-table`)
  scollegata dai set statici usati da `chain banned`/`chain mcp` su questo
  host (bug pre-esistente, vedi `infra/nftables/README.md`). Questa azione
  opera solo su un set IPv6 statico dedicato (`f2b-mcp-ale`, dichiarato in
  `/etc/nftables/banned.conf`), letto da `chain mcp` in
  `infra/nftables/mcp.conf`.

## Test reali eseguiti

- `fail2ban-regex` sul filtro → PASS (8/8 match reali).
- Generate 6 richieste realmente abusive contro il server → ban scattato,
  IP inserito nel set `f2b-mcp-ale` (verificato con `nft list set`).
- Limite noto: i test sono stati eseguiti dalla STESSA macchina che ospita
  il server, quindi il traffico verso il proprio indirizzo ygg0 viene
  instradato internamente via `iif lo`, che intercetta il pacchetto prima
  della `chain mcp` (bypassa il drop). Questo limite è stato superato con
  una verifica da un peer Yggdrasil reale, `server_alex`
  (`201:27c:546:5df7:176:95f3:c909:6834`): MCP raggiungibile prima del ban,
  sei richieste rifiutate (HTTP 421), indirizzo inserito nel set e
  connessione effettivamente bloccata (timeout); dopo l'unban il peer ha
  nuovamente raggiunto il servizio. La stessa prova è stata ripetuta dopo
  il reboot.
