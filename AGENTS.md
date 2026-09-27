# AGENTS.md — Regole operative per qualsiasi agente

> Questo file è la PRIMA istruzione da leggere per qualsiasi agente (umano o
> AI) che lavora su questo repository. Se qualcosa non è chiaro dopo aver
> letto questo file, `README.md` e `memoria/ARCHITETTURA.md`, l'agente NON
> deve inventare la struttura: deve fermarsi e chiedere.

## 0. Lettura obbligatoria prima di lavorare

Prima di qualsiasi modifica, in quest'ordine:

1. `AGENTS.md` (questo file).
2. `README.md`.
3. `memoria/ARCHITETTURA.md`.

`memoria/ARCHITETTURA.md` descrive l'architettura della fabbrica agentica ALE
(nodi, orchestratore, isolamento clienti, CORE gatekeeper). Questo file
(`AGENTS.md`) descrive invece come si lavora su QUESTO repository Git: rami,
struttura delle cartelle, regole di sicurezza per le configurazioni live.
Sono due livelli distinti e complementari — non in conflitto.

## 1. `main` = stato stabile

`main` contiene SOLO stato stabile, documentazione approvata e configurazioni
già verificate. Gli agenti NON devono sviluppare direttamente su `main`.

Ogni modifica avviene in un ramo dedicato, poi viene integrata in `main` solo
dopo verifica esplicita (revisione + test), mai automaticamente.

## 2. Convenzione dei rami

I rami di lavoro si creano per area, con questi prefissi:

```
feat/apache-<nome>
feat/nftables-<nome>
feat/ollama-guard-<nome>
feat/cloudflared-<nome>
feat/mcp-<nome>

fix/<nome>
docs/<nome>
```

`<nome>` descrive brevemente la modifica (es. `feat/ollama-guard-gpu-thresholds`,
`fix/apache-vhost-redirect`, `docs/nftables-runbook`).

## 3. Struttura logica del repository

```
core/
  safety/
    ollama-guard/       # sorgente del watchdog Ollama (Rust), copia versionata
  mcp/                  # server/client MCP (Python, SDK ufficiale v2), read-only

infra/
  apache2/               # configurazioni Apache2 versionate (vhost, template)
  nftables/              # regole nftables versionate (template, incl. mcp.conf)
  systemd/               # unit file systemd versionate (template, incl. mcp-ale.service)
  cloudflared/           # configurazione Cloudflare Tunnel versionata (senza token/credenziali)
  fail2ban/              # filter.d/jail.d/action.d versionati (template, incl. mcp-ale)
  falco/                 # regole custom Falco versionate (template, incl. mcp-ale-security.yaml)

scripts/                 # script di deploy/backup/rollback controllati
  mcp-ale/               # script di avvio/risoluzione IPv6 usati dalla unit mcp-ale.service
memoria/                 # memoria architetturale e decisionale (vedi ARCHITETTURA.md)
```

**Importante — non confondere due livelli diversi:**
- Questo `core/`/`infra/` (repository Git) contiene copie/template/documentazione
  versionata delle configurazioni infrastrutturali (Apache, nftables, systemd,
  Ollama-guard, Cloudflare) e gli script controllati per applicarle.
- Il `core/` descritto in `memoria/ARCHITETTURA.md` è invece la struttura live
  della piattaforma ALE sul server (moduli, template, agenti, librerie per i
  clienti). Sono namespace distinti con scopi distinti.

## 4. Un agente modifica solo la propria area

Ogni agente specializzato lavora esclusivamente nella cartella di propria
competenza. Esempio:

- agente Apache → `infra/apache2/`
- agente nftables → `infra/nftables/`
- agente ollama-guard → `core/safety/ollama-guard/`
- agente Cloudflare → `infra/cloudflared/`
- agente MCP (server/client) → `core/mcp/`
- agente MCP (firewall/hardening) → `infra/nftables/mcp.conf`,
  `infra/fail2ban/` (jail `mcp-ale`), `infra/falco/rules.d/mcp-ale-security.yaml`,
  `infra/systemd/mcp-ale.service`, `scripts/mcp-ale/`

Il servizio MCP (`mcp-ale`) è raggiungibile SOLO tramite l'interfaccia
Yggdrasil (`ygg0`), mai da LAN/Internet: qualsiasi modifica che allarghi
l'esposizione di rete richiede lo stesso protocollo backup→validazione→
test→rollback descritto al punto 5, applicato con la massima cautela
perché tocca `chain input` del firewall condiviso con tutti gli altri
servizi.

Un agente non deve modificare file al di fuori della propria area senza una
ragione esplicita e documentata nel messaggio di commit.

## 5. Regole per le configurazioni LIVE

I file reali in produzione (es. `/etc/apache2/`, `/etc/nftables.conf`,
`/etc/systemd/system/`, ecc.) sono LIVE: vivono sul server, NON in questo
repository. Questo repository NON deve mai trasformare `/etc` in un
repository Git, e nessuno script deve spostare/rinominare file live come
effetto collaterale di un'operazione di organizzazione del repo.

Prima di modificare qualunque configurazione LIVE, un agente deve, in ordine:

1. **Backup**: copiare il file/unit live in un percorso di backup datato
   prima di toccarlo.
2. **Verifica configurazione**: validare la sintassi (es. `nginx -t`
   equivalente, `systemd-analyze verify`, `nft -c -f`, ecc.) prima di
   applicarla.
3. **Test**: eseguire un test reale e mirato del comportamento atteso.
4. **Rollback previsto**: avere pronto e testato il modo per tornare
   rapidamente alla configurazione precedente in caso di problema.

Il repository conserva la copia versionata "di riferimento" (in
`infra/`/`core/`) e lo script di deploy controllato (in `scripts/`); il
deploy reale sul server resta un'azione esplicita, mai implicita.

## 6. Cosa NON deve mai finire nel Git

Non commitare mai, in nessun ramo:

- password
- API key
- token
- chiavi private
- credenziali Cloudflare (incluso il tunnel token)
- secret in generale
- file `.env` reali con valori veri

Se un template di configurazione richiede un segreto, il repository contiene
solo un placeholder (es. `CLOUDFLARE_TUNNEL_TOKEN=__SET_ME__`) e il segreto
reale viene fornito fuori da Git (variabile d'ambiente, vault, file locale
escluso da `.gitignore`).

## 7. Flusso commit → push → integrazione

1. Sviluppo e test avvengono sul ramo dedicato.
2. `commit` sul ramo, con messaggio che spiega chiaramente il sottosistema
   toccato (vedi punto 8).
3. `push` del ramo.
4. Solo dopo verifica (revisione + test reali, non solo `cargo test`/lint) si
   integra il ramo in `main`.

Nessuna modifica arriva in `main` senza essere prima passata da un ramo e da
una verifica esplicita.

## 8. Messaggi di commit

Ogni commit deve indicare chiaramente quale sottosistema cambia, con prefisso
coerente all'area, ad esempio:

```
feat(ollama-guard): aggiunge soglia configurabile per il resume termico
fix(nftables): corregge regola di inoltro per la porta 8443
docs(apache): documenta procedura di rollback vhost
```

## 9. Se la struttura non è chiara

Un agente che non conosce la struttura del repository NON deve inventarla.
Deve leggere `AGENTS.md` e `memoria/ARCHITETTURA.md`. Se dopo la lettura resta
un dubbio reale (es. una nuova area non ancora prevista da nessun prefisso di
ramo), l'agente deve segnalarlo esplicitamente invece di procedere per
analogia.
