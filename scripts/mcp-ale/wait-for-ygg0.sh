#!/bin/bash
# Attende (con timeout) che l'interfaccia ygg0 esista e abbia un indirizzo
# IPv6 globale assegnato, prima di far partire il server MCP. Nessun comando
# amministrativo: solo lettura dello stato di rete (ip -6 addr show).
set -euo pipefail

TIMEOUT_SECONDS=60
INTERVAL_SECONDS=1
elapsed=0

while [ "$elapsed" -lt "$TIMEOUT_SECONDS" ]; do
    ip6=$(ip -6 -o addr show ygg0 scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1)
    if [ -n "$ip6" ]; then
        echo "ygg0 pronta con indirizzo IPv6: $ip6"
        exit 0
    fi
    sleep "$INTERVAL_SECONDS"
    elapsed=$((elapsed + INTERVAL_SECONDS))
done

echo "ERRORE: ygg0 non ha un indirizzo IPv6 globale dopo ${TIMEOUT_SECONDS}s" >&2
exit 1
