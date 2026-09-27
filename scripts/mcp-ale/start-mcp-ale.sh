#!/bin/bash
# Wrapper di avvio del servizio MCP ALE.
# - Attende che ygg0 abbia un indirizzo IPv6 globale (mai avvia il bind su
#   un indirizzo non ancora pronto al boot).
# - Risolve l'indirizzo IPv6 REALE di ygg0 a runtime (non hardcoded nella
#   unit systemd): se Yggdrasil dovesse rigenerare l'indirizzo, il servizio
#   si adatta al riavvio successivo invece di restare bindato a un
#   indirizzo IPv6 non più valido.
# - Non usa MAI 0.0.0.0/::/eth0/wlan: solo l'indirizzo reale di ygg0.
set -euo pipefail

MCP_PORT="${MCP_PORT:-8811}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/wait-for-ygg0.sh"

YGG_IP=$(ip -6 -o addr show ygg0 scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1)
if [ -z "$YGG_IP" ]; then
    echo "ERRORE: impossibile determinare l'indirizzo IPv6 di ygg0" >&2
    exit 1
fi

export MCP_HOST="$YGG_IP"
export MCP_PORT
export MCP_ALLOWED_HOST="[${YGG_IP}]:${MCP_PORT}"

echo "Avvio MCP ALE su [$YGG_IP]:$MCP_PORT (SOLO ygg0, DNS-rebinding protection attiva)"

exec "$SCRIPT_DIR/../.venv/bin/python" "$SCRIPT_DIR/../server/node_observability_server.py"
