"""MCP server minimale, SOLO read-only, per il progetto ALE.

Espone esattamente 3 tool, tutti di sola lettura:
  - node_status:   hostname, uptime, load average, memoria libera
  - ollama_status:  verifica healthy/unhealthy di Ollama via HTTP locale
  - gpu_status:     temperatura/utilizzo/VRAM letti da nvidia-smi

Regole non negoziabili (vedi core/mcp/README.md e AGENTS.md):
  - NESSUNA shell arbitraria: ogni comando esterno è fisso, a compile-time,
    senza input testuale proveniente dal chiamante MCP.
  - NESSUN tool generico tipo `execute_command`.
  - NESSUNA scrittura sul filesystem.
  - NESSUN `sudo`: tutte le informazioni lette qui sono accessibili senza
    privilegi elevati (letture di sistema, HTTP locale, nvidia-smi).
  - Fail closed: se una lettura fallisce, il tool riporta lo stato di
    errore/unhealthy, non solleva un'eccezione che nasconde il problema né
    inventa un valore.

Usa l'SDK ufficiale MCP Python v2 (`mcp.server.mcpserver.MCPServer`), NON i
vecchi esempi FastMCP v1.
"""

from __future__ import annotations

import os
import socket
import subprocess
from dataclasses import dataclass

import httpx2
from mcp.server.mcpserver import MCPServer

# URL fisso, solo loopback: coerente con ollama-guard (core/safety/ollama-guard),
# non deve mai dipendere da un endpoint raggiungibile dall'esterno.
OLLAMA_URL = "http://127.0.0.1:11434/api/tags"
OLLAMA_TIMEOUT_SECONDS = 3.0

# Comando fisso, argomenti fissi, decisi a compile-time: nessun testo esterno
# finisce negli argomenti di nvidia-smi (stessa regola già adottata in
# core/safety/ollama-guard/src/gpu.rs).
NVIDIA_SMI_ARGS = [
    "nvidia-smi",
    "--query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total,power.draw",
    "--format=csv,noheader,nounits",
    "-i",
    "0",
]
NVIDIA_SMI_TIMEOUT_SECONDS = 5.0

mcp = MCPServer(
    name="ale-node-observability",
    title="ALE Node Observability (read-only)",
    instructions=(
        "Fornisce SOLO tool read-only per osservare lo stato di un nodo ALE: "
        "node_status, ollama_status, gpu_status. Nessuna scrittura, nessuna "
        "esecuzione di comandi arbitrari, nessun privilegio elevato."
    ),
)


@dataclass
class GpuStats:
    temperature_c: int
    utilization_pct: int
    memory_used_mib: int
    memory_total_mib: int
    power_draw_w: float


def _read_load_average() -> tuple[float, float, float]:
    """Legge il load average a 1/5/15 minuti (`os.getloadavg`, sola lettura)."""
    return os.getloadavg()


def _read_uptime_seconds() -> float:
    """Legge l'uptime del sistema da `/proc/uptime` (sola lettura, Linux)."""
    with open("/proc/uptime", "r", encoding="utf-8") as f:
        return float(f.readline().split()[0])


def _read_free_memory_mib() -> tuple[int, int]:
    """Legge memoria libera/totale in MiB da `/proc/meminfo` (sola lettura)."""
    values: dict[str, int] = {}
    with open("/proc/meminfo", "r", encoding="utf-8") as f:
        for line in f:
            parts = line.split(":", 1)
            if len(parts) != 2:
                continue
            key = parts[0].strip()
            if key in ("MemTotal", "MemAvailable"):
                # Valori in kB nel file, li convertiamo in MiB.
                kib = int(parts[1].strip().split()[0])
                values[key] = kib // 1024
    total = values.get("MemTotal", 0)
    available = values.get("MemAvailable", 0)
    return available, total


@mcp.tool()
def node_status() -> dict:
    """Stato di base del nodo: hostname, uptime, load average, memoria libera.

    Sola lettura: nessuna scrittura sul filesystem, nessun comando esterno.
    """
    load1, load5, load15 = _read_load_average()
    free_mib, total_mib = _read_free_memory_mib()
    return {
        "hostname": socket.gethostname(),
        "uptime_seconds": round(_read_uptime_seconds(), 1),
        "load_average": {
            "1min": load1,
            "5min": load5,
            "15min": load15,
        },
        "memory": {
            "free_mib": free_mib,
            "total_mib": total_mib,
        },
    }


@mcp.tool()
async def ollama_status() -> dict:
    """Verifica se Ollama risponde su http://127.0.0.1:11434/api/tags.

    Indipendente da ollama-guard: usa una richiesta HTTP diretta e propria,
    con timeout breve. Fail closed: qualunque errore (connessione rifiutata,
    timeout, stato HTTP non 2xx) viene riportato come "unhealthy", mai come
    eccezione non gestita.
    """
    try:
        async with httpx2.AsyncClient() as client:
            response = await client.get(OLLAMA_URL, timeout=OLLAMA_TIMEOUT_SECONDS)
        if response.status_code == 200:
            return {"status": "healthy", "http_status": response.status_code}
        return {
            "status": "unhealthy",
            "http_status": response.status_code,
            "reason": f"risposta HTTP inattesa: {response.status_code}",
        }
    except Exception as e:  # fail closed: qualunque errore -> unhealthy
        return {"status": "unhealthy", "reason": str(e)}


def _parse_nvidia_smi_line(line: str) -> GpuStats:
    fields = [f.strip() for f in line.split(",")]
    if len(fields) != 5:
        raise ValueError(f"riga nvidia-smi non nel formato atteso: {line!r}")
    temperature_c, utilization_pct, memory_used_mib, memory_total_mib, power_draw_w = fields
    return GpuStats(
        temperature_c=int(temperature_c),
        utilization_pct=int(utilization_pct),
        memory_used_mib=int(memory_used_mib),
        memory_total_mib=int(memory_total_mib),
        power_draw_w=float(power_draw_w),
    )


@mcp.tool()
def gpu_status() -> dict:
    """Legge temperatura/utilizzo/VRAM/power draw della GPU tramite nvidia-smi.

    Comando fisso (vedi NVIDIA_SMI_ARGS), nessun input esterno negli
    argomenti. Fail closed: se nvidia-smi non è disponibile o l'output non è
    parsabile, ritorna uno stato di errore esplicito.
    """
    try:
        result = subprocess.run(
            NVIDIA_SMI_ARGS,
            capture_output=True,
            text=True,
            timeout=NVIDIA_SMI_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"status": "error", "reason": f"impossibile eseguire nvidia-smi: {e}"}

    if result.returncode != 0:
        return {
            "status": "error",
            "reason": f"nvidia-smi ha restituito uno stato di errore: {result.stderr.strip()}",
        }

    first_line = result.stdout.strip().splitlines()[0] if result.stdout.strip() else ""
    if not first_line:
        return {"status": "error", "reason": "output di nvidia-smi vuoto"}

    try:
        stats = _parse_nvidia_smi_line(first_line)
    except ValueError as e:
        return {"status": "error", "reason": str(e)}

    return {
        "status": "ok",
        "temperature_c": stats.temperature_c,
        "utilization_pct": stats.utilization_pct,
        "memory_used_mib": stats.memory_used_mib,
        "memory_total_mib": stats.memory_total_mib,
        "power_draw_w": stats.power_draw_w,
    }


if __name__ == "__main__":
    # Streamable HTTP, solo su localhost per il primo test (vedi README.md).
    # Porta scelta per non collidere con altri servizi già in ascolto sulla
    # macchina; documentata anche nel client di test.
    mcp.run(transport="streamable-http", host="127.0.0.1", port=8811)
