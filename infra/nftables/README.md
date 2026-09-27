# nftables — MCP ALE

Questo file è una **copia documentata** (template/riferimento) del modulo
live `/etc/nftables/mcp.conf`. Il file reale in produzione NON è tracciato
in questo repo (regola AGENTS.md: `/etc/...` resta live, il repo contiene
copie/template).

## Come si integra nella struttura modulare esistente

Il firewall di ALE è modulare: `/etc/nftables/core.conf` (= `/etc/nftables.conf`
via symlink) include un file `.conf` per servizio (`apache.conf`, `api.conf`,
`db.conf`, `ygg0.conf`, ecc.), ognuno con la propria `chain` richiamata da
`chain input` tramite `jump <nome>`.

Per MCP sono state aggiunte SOLO queste due righe a `core.conf`
(nessun'altra chain toccata):

```
jump mcp
include "/etc/nftables/mcp.conf"
```

## Cosa fa `mcp.conf`

- Droppa il traffico dai sorgenti IPv6 bannati dalla jail Fail2Ban dedicata
  `mcp-ale` (set `f2b-mcp-ale`, dichiarato in `/etc/nftables/banned.conf`).
- Accetta il servizio MCP (porta TCP 8811) **solo** dall'interfaccia
  `ygg0` (Yggdrasil). `tun0` non è consentita. Nessuna regola qui accetta da altre interfacce:
  il servizio è quindi irraggiungibile da LAN/Internet per costruzione.

Il set IPv6 statico richiesto dalla jail Fail2Ban è rappresentato nel
template [f2b-mcp-ale-set.conf](./f2b-mcp-ale-set.conf) e va incluso nel
file globale `banned.conf` caricato da `core.conf`.

## Procedura di applicazione (già eseguita in produzione)

1. Backup di `core.conf` e `banned.conf` prima di ogni modifica.
2. Validazione sintattica: `sudo -n nft -c -f /etc/nftables/core.conf` → deve
   dare PASS prima di qualsiasi apply.
3. Rollback temporizzato (2 minuti) via `systemd-run` che ripristina il
   backup automaticamente se non annullato:
   `systemd-run --unit=mcp-nft-rollback --on-active=120 <script-rollback>`.
4. Apply reale: `sudo -n systemctl restart nftables`. Il restart carica
   l'intero ruleset e attiva il drop-in
   `infra/systemd/nftables.service.d/mcp-ale-fail2ban-resync.conf`, che
   accoda senza bloccare un riavvio di Fail2Ban dopo il flush, per
   ricreare nei set i ban attivi salvati nel database di Fail2Ban.
   Evitare `nft -f` diretto per gli aggiornamenti operativi: non passa
   da systemd e quindi non attiva il resync automatico.
5. Verificare SSH, Yggdrasil e che Fail2Ban sia attivo e i ban salvati
   siano nuovamente presenti nei set.
6. Se tutto OK, cancellazione del timer di rollback.

## Bug reale scoperto (pre-esistente, non introdotto da MCP)

Il `banaction` di default di Fail2Ban su questo host (`nftables-multiport`)
usa una tabella dinamica (`f2b-table`) scollegata dai set statici
(`f2b-sshd`, `f2b-HONEYPOT-SCAN`, ecc.) referenziati da `chain banned` in
`core.conf`. Ogni reload di nftables (che comincia con `flush ruleset`)
distrugge `f2b-table`, disallineando i ban attivi dalla realtà del
firewall — bug che affligge ANCHE la jail `honeypot-scan` pre-esistente,
non solo MCP. Per la nuova jail `mcp-ale` è stata scritta un'azione
Fail2Ban dedicata (`mcp-ale-nftables`, vedi `infra/fail2ban/`) che opera
solo su un set statico proprio (`f2b-mcp-ale`), per non dipendere da
`f2b-table`. Le jail esistenti NON sono state toccate (fuori perimetro).

**Nota operativa**: usare `systemctl restart nftables`, non un `nft -f`
manuale; il drop-in dedicato riavvia Fail2Ban in modalità non bloccante
ed evita il deadlock di ordinamento `After=nftables.service`. È stato
verificato che un ban reale sopravvive al restart di nftables e che SSH
e Yggdrasil rimangono attivi.
