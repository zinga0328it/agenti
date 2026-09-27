# agenti
Laboratorio R&amp;D per agenti AI distribuiti: architettura multi-nodo isolata, FastAPI, Yggdrasil, broker/relay, sicurezza fail-closed, Falco, honeypot e memoria decisionale dell’agente architetto.

## Come lavorano gli agenti

Qualsiasi agente (umano o AI) che opera su questo repository deve prima
leggere [`AGENTS.md`](./AGENTS.md): definisce i rami di lavoro per area
(`feat/apache-*`, `feat/nftables-*`, `feat/ollama-guard-*`,
`feat/cloudflared-*`, `feat/mcp-*`, `fix/*`, `docs/*`), la struttura logica
del repository (`core/`, `infra/`, `scripts/`, `memoria/`), le regole di
sicurezza per le configurazioni LIVE (backup, verifica, test, rollback prima
di ogni modifica) e cosa non deve mai finire in Git (password, token,
chiavi, secret). `main` contiene solo stato stabile: nessuno sviluppa
direttamente su `main`.
