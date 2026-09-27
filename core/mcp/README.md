# core/mcp — MCP server/client per il progetto ALE

> Prima di lavorare qui, leggi [`AGENTS.md`](../../AGENTS.md) e
> [`memoria/ARCHITETTURA.md`](../../memoria/ARCHITETTURA.md) nella root del
> repository. Questo modulo appartiene all'area MCP (`feat/mcp-*`).

## Cos'è

Un MCP server minimale, **read-only**, che espone informazioni di
osservabilità su un nodo ALE, pensato come primo mattone per il futuro
orchestratore MCP. Usa l'**SDK ufficiale MCP Python v2** (pacchetto `mcp`,
`MCPServer` da `mcp.server.mcpserver`), **non** i vecchi esempi FastMCP v1.

## Struttura

```
core/mcp/
├── README.md                 # questo file
├── requirements.txt           # dipendenze pin-nate (mcp, pytest)
├── server/
│   ├── node_observability_server.py   # il server MCP
│   ├── test_node_observability_server.py  # test automatici (pytest + anyio)
│   └── conftest.py            # fixture pytest (anyio_backend)
└── client/
    └── test_client.py         # client MCP di test end-to-end
```

## Tool esposti (SOLO lettura)

| Tool             | Cosa fa                                                            |
|------------------|---------------------------------------------------------------------|
| `node_status`    | hostname, uptime, load average, memoria libera/totale (letture di sistema) |
| `ollama_status`  | verifica `http://127.0.0.1:11434/api/tags`, ritorna healthy/unhealthy |
| `gpu_status`     | temperatura, utilizzo, VRAM, power draw letti da `nvidia-smi`         |

### Regole non negoziabili (vedi anche `AGENTS.md`)

- **Nessuna shell arbitraria**: ogni comando esterno (`nvidia-smi`) è fisso,
  con argomenti decisi a compile-time, mai costruiti da input del chiamante.
- **Nessun tool generico** tipo `execute_command`.
- **Nessuna scrittura sul filesystem**: tutti e 3 i tool sono di sola
  lettura.
- **Nessun `sudo`**: tutte le informazioni esposte sono leggibili senza
  privilegi elevati.
- **Fail closed**: un errore (es. Ollama giù, nvidia-smi assente) produce un
  payload strutturato con stato di errore/unhealthy, mai un'eccezione MCP
  non gestita che nasconde il problema.

Un test automatico (`test_no_write_or_arbitrary_execution_tool_is_ever_exposed`)
verifica esplicitamente che nessun tool con nomi tipo `execute`/`shell`/
`write_file`/`delete`/`run_command` venga mai esposto.

## Setup locale

```bash
cd core/mcp
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## Eseguire il server (solo localhost)

```bash
source .venv/bin/activate
python server/node_observability_server.py
# Streamable HTTP su http://127.0.0.1:8811/mcp
```

La porta (8811) è stata scelta perché altre porte comuni (es. 8765) erano già
occupate da altri servizi sulla stessa macchina; verificare sempre con `ss
-tln` prima di scegliere una porta diversa in un altro ambiente.

## Eseguire il client di test end-to-end

Con il server già avviato in un altro terminale:

```bash
source .venv/bin/activate
python client/test_client.py
# oppure specificando l'URL:
python client/test_client.py http://127.0.0.1:8811/mcp
```

Il client si connette, esegue `initialize`, elenca i tool (`tools/list`),
chiama i 3 tool e stampa il risultato JSON di ciascuno. Fallisce
esplicitamente (`AssertionError`) se il server espone tool diversi da quelli
attesi.

## Test automatici (SDK MCP, senza rete)

```bash
source .venv/bin/activate
pytest server/ -v
```

I test usano l'helper ufficiale dell'SDK `mcp.client._memory.InMemoryTransport`
per collegare un vero `ClientSession` al vero `MCPServer` senza aprire alcuna
porta di rete, esercitando lo stesso protocollo (`initialize`, `tools/list`,
`tools/call`) del client reale.

## Stato

Primo test locale (solo `127.0.0.1`), non ancora esposto in rete, non ancora
collegato all'orchestratore ALE descritto in `memoria/ARCHITETTURA.md`. Nessuna
modifica ad Ollama, Apache, nftables o Yggdrasil è stata fatta per questo
modulo.
