# Falco — MCP ALE / ollama-guard / Ollama

Copia documentata del modulo custom `rules.d/mcp-ale-security.yaml`, che va
installato in `/etc/falco/rules.d/` (directory già inclusa da
`rules_files:` in `/etc/falco/falco.yaml`, nessuna modifica necessaria a
quel file).

## Regole incluse

1. **Shell inattesa generata dal processo MCP ALE** (CRITICAL).
2. **Esecuzione di programma non previsto dal processo MCP ALE** (WARNING)
   — whitelist esplicita di `nvidia-smi` come unico figlio legittimo.
3. **Processo figlio sospetto di ollama-guard/ollama** (WARNING).
4. **Scrittura inattesa nei file MCP** (CRITICAL).
5. **Modifica della unit systemd MCP/ollama-guard** (CRITICAL).

Per ora tutte le regole sono SOLO alert (nessuna azione di kill
automatico), come richiesto.

## Bug reale scoperto e corretto

La regola 2 inizialmente generava falsi positivi su ogni chiamata reale
al tool `gpu_status` (che esegue legittimamente `nvidia-smi`): il modulo
`subprocess` di Python genera un evento `execve` transitorio interno
(`posix_spawn`) in cui il "figlio" ha ancora la cmdline del padre
(compreso `node_observability_server.py`) prima dell'exec effettivo del
comando target. Il DSL di Falco NON supporta il confronto diretto fra due
campi (`proc.cmdline=proc.pcmdline` produce il warning
`LOAD_COMPILE_CONDITION`, poiché lo interpreta come confronto con una
stringa costante). Corretto escludendo esplicitamente
`proc.cmdline contains "node_observability_server.py"` invece del
confronto fra campi.

## Test reali eseguiti

- `falco --dry-run` → schema OK, nessun warning sulle regole custom.
- Chiamata reale `gpu_status` via client MCP → nessun alert (falso
  positivo risolto, confermato via `journalctl -u falco-modern-bpf`).
- Shell simulata con cmdline contenente il marker del processo MCP →
  alert CRITICAL "Shell inattesa" scattato correttamente.
- Scrittura reale (append + ripristino immediato da backup) sulla unit
  `mcp-ale.service` → alert CRITICAL "Modifica della unit systemd"
  scattato correttamente, sia sulla scrittura sia sul successivo
  ripristino.
