# Memoria dell'agente architetto

## Obiettivo
Architettura distribuita e compartimentata: la compromissione di un nodo esposto non deve concedere automaticamente accesso ai nodi successivi, al backend o al database.

## Modello iniziale

```text
Internet / client
      |
      v
PC1 - Edge / Apache2
      |
      v
PC2 - Relay / Broker
      |
      v
Rete privata Yggdrasil
      |
      v
Backend / server centrale / DB
```

## Principi emersi

### Compartimentazione
PC1 espone Apache2, ma non contiene il database e non deve esporre direttamente i servizi/sorgenti Python del backend. Ogni nodo conosce solo quanto necessario per comunicare con il livello successivo.

### Identita differenti
PC1 e PC2 utilizzano identita e credenziali differenti. La compromissione di PC1 non deve fornire automaticamente l'identita necessaria per impersonare il relay verso i livelli successivi.

### Difesa a strati
Una chiave da sola non equivale ad accesso. L'autorizzazione deriva dalla combinazione di percorso di rete, nftables, identita del nodo, credenziale valida e autorizzazione applicativa.

### Yggdrasil
I servizi interni sono separati dalla superficie Internet e raggiungibili attraverso la rete privata Yggdrasil secondo policy esplicite.

### nftables allowlist
Default deny sui servizi interni. Sono ammessi esclusivamente i flussi previsti dall'architettura e provenienti dai nodi/reti autorizzati.

### Rotazione delle chiavi
Le chiavi operative vengono ruotate automaticamente ogni giorno. La rotazione limita la vita utile di una credenziale sottratta, ma non sostituisce revoca e isolamento.

### Falco
Falco opera su PC1 e PC2 come sensore runtime per individuare comportamenti non previsti dall'architettura, processi anomali, shell inattese e accessi sospetti.

### Canary e deception
PC1 e PC2 contengono sensori/esche. Porte che non appartengono a flussi legittimi possono generare eventi di sicurezza e attivare Fail2ban/nftables. Le esche sono un ulteriore sensore e non devono essere considerate infallibili.

### Fail-closed
Quando viene rilevata una compromissione credibile, il sistema deve poter revocare le comunicazioni e isolare i nodi interessati. Un nodo precedentemente autorizzato non rimane automaticamente affidabile.

### Telegram
Telegram Bot serve per avvisare l'operatore. Il contenimento automatico non deve dipendere dalla disponibilita di Telegram o dall'intervento umano.

### Nodo compromesso
Se un attaccante ottiene root su un nodo, assumiamo che possa osservare o manipolare cio che quel nodo possiede su disco e RAM. I livelli successivi devono quindi mantenere una propria barriera di fiducia indipendente.

## Regola dell'agente architetto

> Non chiedere soltanto se una chiave e sicura. Chiedere: se questo nodo viene completamente compromesso, quale altro nodo puo raggiungere, con quale identita e con quali privilegi?

## Questioni aperte
- Attestazione dell'identita PC1 -> PC2 senza dipendere soltanto da un segreto copiabile.
- Distribuzione e revoca automatica delle credenziali.
- Autorita del controller che ordina l'isolamento.
- Protezione da falsi eventi usati per isolare arbitrariamente altri nodi.
- Continuita del servizio quando PC1/PC2 vengono messi in quarantena.
- Eventuali nodi puliti di standby e failover.
- Protocollo FastAPI tra relay e insieme rigoroso dei messaggi consentiti.
