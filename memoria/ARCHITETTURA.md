# Memoria ALE — Architettura della fabbrica agentica

> Stato: decisioni consolidate dalle prime 11 domande architetturali.
> Questo documento descrive principi e struttura. Non deve contenere password, token o chiavi private.

## 1. Obiettivo
ALE e una fabbrica agentica capace di ricevere dal cliente richieste per siti e gestionali, interpretarle, riutilizzare componenti verificati e creare dinamicamente lo sciame di specialisti necessario.

Principio: **sciame dinamico, comando centralizzato e confini decentralizzati.**

## 2. Percorso della richiesta

```text
CLIENTE
  |
  v
ALEX
Apache2 + SOLO sito statico Servicess
  |
  v
ZINGA
FastAPI Relay / Broker
  |
  v
AAA
DB + Python SOLO di Servicess
validazione della richiesta
  |
  v
ALE
orchestratore + fabbrica clienti
```

Il cliente entra sempre dal sito Servicess su ALEX. ALE non e il punto di ingresso pubblico diretto del cliente.

### ALEX
- Apache2.
- Solo frontend/sito statico Servicess.
- Nessuna logica Python, database o orchestrazione agenti.

### ZINGA
- Relay/Broker FastAPI.
- Autentica il passaggio in ingresso.
- Traduce/separa le credenziali tra hop.
- Non e un semplice relay cieco.

### AAA
- Database di Servicess.
- Python di Servicess.
- Riceve le richieste provenienti dal percorso autorizzato e le valida prima dell'inoltro verso ALE.
- L'input cliente rimane dato: non deve diventare arbitrariamente shell, SQL o codice eseguibile.
- Usa formati/schema consentiti e separazione tra dati e comandi.
- Input non ammessi vengono rifiutati e possono attivare la procedura di sicurezza prevista.

### ALE
- Server grande della fabbrica.
- Ospita orchestrazione, core verificato e domini dei clienti.
- Ospita Apache/configurazioni, applicazioni e database dei clienti secondo isolamento definito sotto.

## Servizi MCP ALE — rete e avvio

Il server MCP read-only è eseguito dall'unità `mcp-ale.service` come utente
non privilegiato dedicato. Ascolta sulla porta TCP 8811, bindata
all'indirizzo IPv6 reale di `ygg0` risolto durante l'avvio; la policy
nftables accetta questa porta esclusivamente con `iifname "ygg0"`.
IPv4/LAN e altre interfacce non sono endpoint MCP consentiti.

Ordine operativo al boot:

1. rete disponibile;
2. Yggdrasil crea `ygg0` e il suo IPv6;
3. nftables carica il ruleset modulare, inclusi set statici Fail2Ban e la
   chain MCP;
4. Fail2Ban avvia le jail e riallinea i ban persistenti con nftables;
5. Falco, Ollama e ollama-guard avviano i rispettivi controlli;
6. MCP attende rete/Yggdrasil e l'indirizzo `ygg0`, quindi avvia il server.

Il drop-in systemd di nftables accoda un riavvio non bloccante di Fail2Ban
quando il ruleset viene riapplicato: `flush ruleset` elimina i set dinamici,
perciò Fail2Ban deve ricrearli dal proprio database persistente. Le
modifiche operative al firewall vanno applicate con `systemctl restart
nftables` (dopo backup, `nft -c -f` e rollback temporizzato), non con un
`nft -f` diretto che bypasserebbe l'hook systemd di sincronizzazione.

## Tool MCP `ollama_guard_status` — canale di stato ollama-guard → MCP

`ollama-guard` (root) pubblica periodicamente il proprio stato interno in
un file JSON leggibile da chiunque ma scrivibile solo da root:

```
ollama-guard (root, RuntimeDirectory=ollama-guard)
    │  scrittura atomica (tmpfile + rename, permessi 0644)
    ▼
/run/ollama-guard/status.json
    │  sola lettura (0644, proprietario root: mcp-ale NON ha permesso di
    │  scrittura, né sulla directory /run/ollama-guard che è 0755 root)
    ▼
mcp-ale.service (utente non privilegiato)
    │
    ▼
tool MCP `ollama_guard_status`
```

Contenuto del file (`core/safety/ollama-guard/src/status.rs`):

```json
{
  "state": "WATCHING",
  "ollama": "healthy",
  "gpu_temperature_c": 42,
  "restarts_10m": 0,
  "last_error": null,
  "updated_at": "2026-09-27T12:41:36.771572803+00:00"
}
```

Stati ammessi per `state`: `WATCHING`, `THERMAL_HOLD`, `FAULT` (il `FAULT`
è "sticky": una volta raggiunto non viene mai sovrascritto da una
transizione di salute o termica; `THERMAL_HOLD` può tornare a `WATCHING`
solo da sé stesso, mai da `FAULT`). La scrittura è sempre atomica: file
temporaneo nella stessa directory (stesso filesystem, `/run` è tmpfs) poi
`rename()`, mai una scrittura in-place che potrebbe lasciare un JSON a metà
se letto in quel preciso istante.

Il tool `ollama_guard_status` (in `core/mcp/server/node_observability_server.py`)
è a sola lettura per costruzione: legge il file con `open(...)` in modalità
`"r"`, non lo modifica mai, non ha e non ha bisogno di `sudo`. Gestisce tre
casi non-`ok` in modo esplicito (fail closed, mai un'eccezione che nasconde
lo stato reale):

- file assente (es. ollama-guard non ancora partito, o mai partito):
  `status: "unavailable"`;
- `updated_at` più vecchio di `OLLAMA_GUARD_STALE_AFTER_SECONDS` (30s,
  ampiamente sopra il ciclo GPU più lento, tipicamente 10s): il watchdog
  potrebbe essere morto/bloccato, `status: "stale"` — non bisogna fidarsi
  di un ultimo stato ottimistico se non viene più aggiornato;
- JSON malformato/illeggibile: `status: "error"`.

Verificato realmente (non solo per costruzione dei permessi) che l'utente
`mcp-ale` non può scrivere `/run/ollama-guard/status.json`: un tentativo
diretto (`sudo -n -u mcp-ale bash -c 'echo x >> ...'`) fallisce con
"Permesso negato", sia a livello di shell sia a livello di regola Falco
dedicata (`infra/falco/rules.d/mcp-ale-security.yaml`): una regola segnala
CRITICAL qualunque scrittura riuscita al file da un processo diverso da
`ollama-guard`, un'altra segnala CRITICAL anche il solo *tentativo negato*
dal kernel (usando la macro Falco `open_file_failed`, perché una `open()`
fallita per permessi non produce un file descriptor valido e la macro
generica `open_write` di Falco non la intercetta).

## Capo Architetto — `qwen2.5-coder:14b-base`

Il modello designato come "Capo Architetto" dello sciame (visione completa
in `/srv/qwen-base/README_AI_VIRUS.md`) è **`qwen2.5-coder:14b-base`**:

```
qwen2.5-coder:14b-base
        ↓
CAPO ARCHITETTO (riceve un obiettivo, sceglie una ACTION esistente)
        ↓
MCP ALE
        ↓
sciame
        ↓
AAA / ZINGA / ALEX
```

Il Capo Architetto **non programma né esegue shell direttamente**: sceglie
un'azione già prevista e compila gli argomenti, il core Rust dello sciame
controlla se l'azione è autorizzata prima di eseguirla (regola già
stabilita: "LLM = decide, Rust = controlla").

### Incidente (2026-09-26/27) e correzione

Il modello era stato inizialmente scaricato e provato con una **seconda
istanza Ollama avviata manualmente**:

```
nohup env OLLAMA_MODELS=/srv/qwen-base/models OLLAMA_HOST=127.0.0.1:11434 ollama serve &
```

Due problemi reali, entrambi confermati durante l'analisi:

1. **Stessa porta** (`127.0.0.1:11434`) della singola istanza `ollama.service`
   già sorvegliata da `ollama-guard`: architetturalmente sbagliato, e in
   pratica una seconda istanza in ascolto sulla stessa porta non può
   coesistere in modo affidabile con la prima.
2. **Competizione per la GPU**: la prima volta il modello si caricava
   correttamente (`offloaded 49/49 layers to GPU`), ma dopo il secondo
   caricamento (con l'altra istanza/modello già a occupare VRAM) Ollama non
   trovava più memoria GPU sufficiente e ripiegava sulla CPU
   (`layers.offload=0`, 100% CPU) — esattamente il sintomo segnalato.

**Correzione applicata**: nessuna seconda istanza Ollama. Il modello è stato
importato nello store standard dell'unica istanza sorvegliata da
`ollama-guard`:

```
sudo cp /srv/qwen-base/models/blobs/sha256-*        /usr/share/ollama/.ollama/models/blobs/
sudo cp /srv/qwen-base/models/manifests/registry.ollama.ai/library/qwen2.5-coder/14b-base \
        /usr/share/ollama/.ollama/models/manifests/registry.ollama.ai/library/qwen2.5-coder/14b-base
sudo chown -R ollama:ollama /usr/share/ollama/.ollama/models/blobs/sha256-8000ab37... /usr/share/ollama/.ollama/models/manifests/registry.ollama.ai/library/qwen2.5-coder
sudo systemctl restart ollama
```

I blob sono identificati per hash SHA-256 (storage content-addressable di
Ollama): due dei quattro blob (template e license) erano già presenti,
condivisi con altri modelli Qwen2 già installati, senza bisogno di
duplicarli.

**Verificato realmente, dopo la correzione:**
- `curl http://127.0.0.1:11434/api/tags` elenca `qwen2.5-coder:14b-base`
  insieme agli altri modelli, su un'unica istanza (`ss -ltnp | grep 11434`
  mostra un solo processo in ascolto);
- una `generate` reale produce nei log di `ollama.service`:
  `llm_load_tensors: offloaded 49/49 layers to GPU`, con `nvidia-smi` che
  mostra ~11 GiB di VRAM occupata durante l'inferenza;
- un test di recovery reale (`systemctl stop ollama` mentre `ollama-guard`
  era attivo) ha mostrato il ciclo atteso: 1/3 → 2/3 fallimenti di health
  check, poi il restart automatico, `restarts_10m` incrementato a 1 in
  `/run/ollama-guard/status.json`, Ollama di nuovo `healthy` in circa 15
  secondi — e il modello, ricaricato su richiesta dopo il riavvio, di nuovo
  con `offloaded 49/49 layers to GPU` (nessuna dipendenza persa dal
  riavvio: il modello vive nello store, non nel processo).

`ollama-guard` non ha richiesto modifiche di codice: il suo health check è
volutamente agnostico rispetto al modello caricato (osserva solo se
l'endpoint HTTP `/api/tags` risponde), quindi sorveglia correttamente
qualunque modello serva l'istanza, incluso il Capo Architetto.

Il tool MCP `ollama_status` (`core/mcp/server/node_observability_server.py`)
è stato esteso per riportare esplicitamente il Capo Architetto configurato
e se risulta disponibile sull'istanza osservata:

```json
{
  "status": "healthy",
  "http_status": 200,
  "capo_architetto_model": "qwen2.5-coder:14b-base",
  "capo_architetto_available": true
}
```

`/srv/qwen-base` resta come copia sorgente/archivio del modello (blob +
manifest originali), ma il suo README è stato aggiornato per vietare
esplicitamente il riavvio manuale via `nohup`: quella procedura è la causa
diretta dell'incidente e non deve più essere usata.

**Nota di scope**: l'MCP server attuale resta un server di *osservazione*
(node/ollama/gpu/ollama-guard status, tutto read-only). Il livello
"orchestratore" che userà realmente il Capo Architetto per scegliere ACTION
e parlare con lo sciame (MCP scritto in Rust, tool per nodo, "DNA Virus")
è la fase successiva descritta in `/srv/qwen-base/README_AI_VIRUS.md` e non
è stato implementato in questo intervento: qui è stato corretto solo il
problema tecnico reale (doppia istanza Ollama / GPU) e reso il modello
osservabile da MCP.

## 3. Orchestratore ALE
Ogni richiesta entra da un solo punto decisionale: **l'Agente Orchestratore**.


```text
richiesta validata
       |
       v
ORCHESTRATORE
       |
       +-- carica memoria ALE
       +-- legge stato reale
       +-- interpreta requisiti
       +-- cerca moduli esistenti
       +-- prepara piano
       +-- stabilisce dipendenze
       +-- decide gli specialisti
       |
       +--------+--------+
       v        v        v
   Frontend   Backend   Database
      Agent     Agent      Agent
       +--------+--------+
                v
          Test / Security
                |
                v
              Deploy
```

Gli specialisti non si auto-attivano. L'orchestratore crea/delega job e stabilisce ordine e dipendenze.

Ogni agente lavora esclusivamente nel proprio dominio di responsabilita e privilegi e comunica tramite le interfacce previste.

## 4. Architetto temporaneo
Se una richiesta richiede una capacita assente o una modifica delle fondamenta, l'orchestratore **non improvvisa**.

```text
ORCHESTRATORE
     |
funzione/capacita non conosciuta
     |
     v
ARCHITETTO TEMPORANEO
     |
     +-- progetta modulo/struttura
     +-- definisce confini di privilegio
     +-- assegna specialisti
     +-- sviluppo
     +-- test
     +-- security audit
     +-- GitHub / Architecture Memory
     |
     v
terminazione Architetto temporaneo
```

**Legge:** l'orchestratore puo scegliere come utilizzare l'infrastruttura, ma non puo cambiare autonomamente le leggi dell'infrastruttura.

In forma sintetica: **l'orchestratore amministra cio che ALE possiede; l'Architetto modifica cio che ALE e.**

## 5. Memoria obbligatoria prima di pianificare
Prima di interpretare la richiesta cliente, l'orchestratore deve conoscere:

1. Topologia: nodi, ruoli, gateway/broker e collegamenti Yggdrasil.
2. Leggi ALE: separazione privilegi, servizi non pubblicabili, flussi consentiti, una responsabilita per agente.
3. Capacita: CPU, RAM, storage e runtime disponibili sui nodi.
4. Catalogo moduli verificati: login, booking, clienti, camere, Telegram OTP, pagamenti, backup, Cloudflare, Apache, database e futuri moduli.
5. Catalogo agenti: competenze, strumenti, nodo consentito e privilegi.
6. Mappa permessi: filesystem, utenti/gruppi Linux, sudo autorizzato, API e servizi raggiungibili/non raggiungibili.
7. Rete e sicurezza: identita/riferimenti Yggdrasil, API interne, policy nftables, autenticazione servizi, Cloudflare Tunnel, Fail2ban. La memoria conserva riferimenti ai secret, non i secret.
8. Stato reale: nodi e servizi attivi, job, capacita occupata, problemi conosciuti.
9. Standard progetto: directory, API, DB, logging, test, naming, GitHub, deploy, rollback e documentazione.
10. Decisioni precedenti: soluzioni, errori e motivazioni architetturali.
11. Vincoli commerciali: servizi acquistati, risorse incluse, extra e limiti contrattuali.
12. Escalation: quando deve essere creato un job per l'Architetto temporaneo.

Sequenza:

```text
CARICA MEMORIA ALE
       |
LEGGI STATO REALE
       |
LEGGI MODULI + AGENTI
       |
APPLICA VINCOLI SICUREZZA
       |
ANALIZZA RICHIESTA
       |
ESISTE GIA TUTTO?
   |           |
  SI          NO
   |           |
workflow    ARCHITETTO
   |        TEMPORANEO
   +-----+-----+
         v
    CREA SCIAME
```

## 6. Struttura piattaforma e clienti

```text
ALE/
├── core/                    # SOLO piattaforma ALE
│   ├── modules/
│   ├── templates/
│   ├── agents/
│   └── libraries/
│
└── clienti/
    ├── hotel-roma/
    │   ├── frontend/
    │   ├── gestionale/
    │   ├── backend/
    │   ├── config/
    │   ├── migrations/
    │   ├── logs/
    │   ├── tests/
    │   └── progetto.yaml
    └── hotel-milano/
        └── ...
```

Ogni cliente costituisce un dominio isolato. Non viene duplicata inutilmente l'intera piattaforma ALE.

## 7. Isolamento per cliente
Ogni cliente possiede almeno:
- utente Linux dedicato;
- directory dedicate;
- applicazione/configurazione dedicate;
- database e ruolo PostgreSQL dedicati;
- credenziali dedicate;
- log e backup separati;
- dominio/tunnel/configurazione separati;
- processi applicativi e permessi separati.

Esempio:

```text
/clienti/hotel-roma/    -> ale_hotel_roma
/clienti/hotel-milano/  -> ale_hotel_milano

PostgreSQL condiviso come motore:
  cliente_hotel_roma    -> hotel_roma_app
  cliente_hotel_milano  -> hotel_milano_app
```

Lo stesso motore PostgreSQL puo essere condiviso, ma non dati, identita e autorizzazioni. Anche Apache2 e Cloudflare possono essere infrastrutture comuni con VirtualHost/tunnel/configurazioni separati.

### Condiviso
- motore PostgreSQL;
- Apache2;
- Yggdrasil;
- sistema agenti;
- template;
- moduli verificati;
- librerie;
- orchestratore.

### Isolato
- identita Linux;
- filesystem;
- applicazione;
- configurazione;
- database/ruolo;
- credenziali;
- log;
- backup;
- dominio/tunnel;
- processi;
- permessi.

**Legge:** **condividere le capacita, mai la fiducia. Ogni cliente possiede un proprio dominio di dati, identita, filesystem e privilegi.**

Gli agenti sono lavoratori temporanei, non proprietari permanenti del cliente. Durante un job ricevono esclusivamente il contesto/privilegio del cliente interessato. Terminato il job, il privilegio temporaneo termina.

## 8. progetto.yaml
`progetto.yaml` e la carta d'identita/version manifest del progetto. Esempio concettuale:

```yaml
cliente: hotel-roma
tipo: hotel

modules:
  booking:
    version: "1.1.0"
    commit: "a84f..."
    checksum: "sha256:..."
  login:
    version: "2.3.1"
  telegram_otp:
    version: "1.4.0"

runtime:
  frontend: apache
  backend: fastapi
  database: postgresql

isolation:
  linux_user: ale_hotel_roma
  database: cliente_hotel_roma

network:
  public: cloudflare
  internal: yggdrasil
```

## 9. CORE versionato e release cliente bloccate
I moduli CORE sono immutabili/versionati:

```text
/core/modules/booking/
├── 1.0.0/
├── 1.1.0/
└── 2.0.0/
```

Ogni progetto fissa precisamente le versioni approvate. Dove applicabile conserva anche commit e checksum.

Il runtime cliente **non dipende da un alias modificabile come `current/`**. La build/release del cliente contiene o risolve in modo immutabile le dipendenze precise approvate.

**Legge:** **il CORE fornisce componenti versionati. Ogni cliente fissa le proprie versioni. Nessun aggiornamento del CORE modifica automaticamente un progetto gia distribuito.**

## 10. Aggiornamento moduli cliente
Una nuova release CORE non aggiorna automaticamente la produzione.

```text
nuova versione CORE
       |
compatibility agent
       |
test progetto cliente
       |
migration se necessaria
       |
security test
       |
approvazione / deploy
       |
progetto.yaml aggiornato
```

Una funzione sviluppata inizialmente per un cliente non diventa automaticamente CORE. Se generalizzabile, viene ripulita, resa generica, testata e sottoposta ad audit prima di essere promossa a nuova release CORE disponibile per altri progetti.

## 11. CORE Gatekeeper — il caveau ALE
Nessun agente di progetto puo scrivere direttamente nel CORE. Il CORE e il caveau della piattaforma.

```text
AGENTI HOTEL-ROMA
        |
        | sviluppano
        v
/staging/core-candidates/
        |
        v
AGENTE TEST
        |
        v
AGENTE SECURITY
        |
        v
AGENTE CORE-GATEKEEPER
        |
        | promozione controllata
        v
/core/modules/
```

### Autorita

```text
orchestratore     -> READ CORE
agenti progetto   -> READ CORE
architetto        -> READ CORE + WRITE STAGING
security agent    -> READ + AUDIT
core-gatekeeper   -> WRITE CORE
```

Nemmeno l'Orchestratore principale puo scrivere direttamente in `/core`.

A livello Linux il confine deve essere reale e non soltanto un'istruzione data all'AI:

```text
/core/
owner: ale-core
write: SOLO core-gatekeeper

/staging/
write: agenti autorizzati

/clienti/hotel-roma/
write: sciame hotel-roma
```

### Gatekeeper deterministico
Il CORE Gatekeeper non e un agente di sviluppo e non deve programmare. Possiede un insieme minimo di operazioni di promozione:

```text
riceve candidate
      |
verifica test
      |
verifica security audit
      |
verifica manifest/checksum
      |
assegna versione
      |
promuove
      |
Git commit/tag
      |
CORE
```

Esempio:

```text
hotel-roma sviluppa
      |
staging/booking-calendar
      |
test
      |
audit
      |
APPROVATO
      |
core/modules/booking-calendar/1.0.0
```

Solo dopo la promozione il modulo diventa patrimonio riutilizzabile di ALE.

### Least privilege del Gatekeeper
Il Gatekeeper non deve possedere un normale `sudo ALL`. Deve avere esclusivamente le operazioni necessarie alla promozione nel CORE. L'identita di amministrazione/bootstrap della macchina resta separata dal Gatekeeper e dallo sciame ordinario.

### Livelli di autorita

```text
LIVELLO 1   Agenti progetto
            lavorano nel recinto cliente

LIVELLO 2   Orchestratore / Architetto
            progettano e delegano

LIVELLO 3   CORE Gatekeeper
            custodisce e promuove il CORE

SOPRA       Amministrazione / bootstrap macchina
            identita separata dallo sciame ordinario
```

**Legge ALE #11:** **Nessun progetto modifica il CORE. Un progetto puo proporre conoscenza; soltanto il Gatekeeper puo promuoverla nel CORE dopo verifica.**

Questo impedisce a un agente compromesso nel recinto di un cliente di avvelenare direttamente componenti condivisi che potrebbero essere distribuiti successivamente ad altri clienti.

## Principi di sicurezza gia stabiliti
- Defense in depth e default deny per i servizi interni.
- Yggdrasil separa la rete interna dalla superficie Internet secondo policy esplicite.
- nftables consente solo i flussi previsti.
- Identita/credenziali differenti tra hop.
- Rotazione credenziali, revoca e isolamento sono controlli distinti.
- Falco e deception/canary possono rilevare comportamenti anomali.
- Fail-closed: un nodo compromesso non resta automaticamente affidabile.
- Telegram e un canale di allerta, non il controllo di sicurezza da cui dipende il contenimento.
- Se un nodo ottiene compromissione root, si assume compromesso cio che quel nodo puo leggere/manipolare; i livelli successivi mantengono barriere indipendenti.

## Prossimo punto da definire
La prossima decisione architetturale deve partire da Q12, mantenendo le prime 11 decisioni come leggi consolidate salvo revisione esplicita.
