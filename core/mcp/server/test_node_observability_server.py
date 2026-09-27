"""Test automatici del server MCP `node_observability_server`.

Usa l'helper ufficiale dell'SDK MCP `InMemoryTransport`
(`mcp.client._memory`) per collegare un `ClientSession` direttamente
all'istanza `MCPServer` senza rete, esercitando lo stesso protocollo
(initialize, tools/list, tools/call) usato dal client reale in
`client/test_client.py`.

Esegui con:
    cd core/mcp && source .venv/bin/activate && pytest -v
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from mcp import ClientSession  # noqa: E402
from mcp.client._memory import InMemoryTransport  # noqa: E402

from node_observability_server import mcp as node_mcp  # noqa: E402

EXPECTED_TOOLS = {"node_status", "ollama_status", "gpu_status"}


@pytest.fixture
async def session():
    """Sessione MCP connessa in-memory al vero server (nessuna rete)."""
    async with InMemoryTransport(node_mcp) as (read_stream, write_stream):
        async with ClientSession(read_stream, write_stream) as s:
            await s.initialize()
            yield s


@pytest.mark.anyio
async def test_tools_list_exposes_exactly_the_three_readonly_tools(session: ClientSession):
    """Il server deve esporre SOLO i 3 tool read-only previsti, nient'altro.

    Questo test è anche una guardia di sicurezza: se in futuro qualcuno
    aggiungesse per errore un tool tipo `execute_command` o una scrittura
    filesystem, questo test deve fallire.
    """
    result = await session.list_tools()
    tool_names = {t.name for t in result.tools}
    assert tool_names == EXPECTED_TOOLS


@pytest.mark.anyio
async def test_node_status_reports_real_hostname_and_memory(session: ClientSession):
    result = await session.call_tool("node_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["hostname"]
    assert payload["memory"]["total_mib"] > 0
    assert "load_average" in payload
    assert payload["uptime_seconds"] >= 0


@pytest.mark.anyio
async def test_ollama_status_reports_healthy_or_unhealthy_never_raises(session: ClientSession):
    """Non importa se Ollama è su o giù in questo momento: il tool deve
    SEMPRE rispondere con un payload strutturato (fail closed), mai con
    un'eccezione MCP non gestita.
    """
    result = await session.call_tool("ollama_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] in ("healthy", "unhealthy")


@pytest.mark.anyio
async def test_gpu_status_reports_real_values_or_explicit_error(session: ClientSession):
    result = await session.call_tool("gpu_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] in ("ok", "error")
    if payload["status"] == "ok":
        assert payload["temperature_c"] > 0
        assert payload["memory_total_mib"] > 0


@pytest.mark.anyio
async def test_no_write_or_arbitrary_execution_tool_is_ever_exposed(session: ClientSession):
    """Guardia esplicita richiesta dai requisiti: nessuna shell arbitraria,
    nessun tool tipo `execute_command`, nessuna scrittura filesystem.
    """
    result = await session.list_tools()
    forbidden_substrings = ("execute", "shell", "write_file", "delete", "run_command")
    for tool in result.tools:
        lowered = tool.name.lower()
        for forbidden in forbidden_substrings:
            assert forbidden not in lowered, f"tool pericoloso trovato: {tool.name}"
