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

import datetime
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from mcp import ClientSession  # noqa: E402
from mcp.client._memory import InMemoryTransport  # noqa: E402

import node_observability_server  # noqa: E402
from node_observability_server import mcp as node_mcp  # noqa: E402

EXPECTED_TOOLS = {"node_status", "ollama_status", "gpu_status", "ollama_guard_status"}


@pytest.fixture
async def session():
    """Sessione MCP connessa in-memory al vero server (nessuna rete)."""
    async with InMemoryTransport(node_mcp) as (read_stream, write_stream):
        async with ClientSession(read_stream, write_stream) as s:
            await s.initialize()
            yield s


@pytest.mark.anyio
async def test_tools_list_exposes_exactly_the_four_readonly_tools(session: ClientSession):
    """Il server deve esporre SOLO i 4 tool read-only previsti, nient'altro.

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
    if payload["status"] == "healthy":
        assert payload["capo_architetto_model"] == node_observability_server.CAPO_ARCHITETTO_MODEL
        assert isinstance(payload["capo_architetto_available"], bool)


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


# ---------------------------------------------------------------------------
# ollama_guard_status: legge SOLO /run/ollama-guard/status.json (mockato nei
# test tramite monkeypatch della costante di modulo, mai un path reale).
# ---------------------------------------------------------------------------


def _write_status_json(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload), encoding="utf-8")


@pytest.mark.anyio
async def test_ollama_guard_status_unavailable_when_file_missing(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    missing = tmp_path / "does-not-exist" / "status.json"
    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(missing))

    result = await session.call_tool("ollama_guard_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] == "unavailable"


@pytest.mark.anyio
async def test_ollama_guard_status_ok_when_fresh(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    status_path = tmp_path / "status.json"
    now = datetime.datetime.now(datetime.timezone.utc)
    _write_status_json(
        status_path,
        {
            "state": "WATCHING",
            "ollama": "healthy",
            "gpu_temperature_c": 41,
            "restarts_10m": 0,
            "last_error": None,
            "updated_at": now.isoformat(),
        },
    )
    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(status_path))

    result = await session.call_tool("ollama_guard_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] == "ok"
    assert payload["guard_state"] == "WATCHING"
    assert payload["ollama"] == "healthy"
    assert payload["gpu_temperature_c"] == 41
    assert payload["restarts_10m"] == 0
    assert payload["last_error"] is None
    assert payload["age_seconds"] < 5.0


@pytest.mark.anyio
async def test_ollama_guard_status_stale_when_old(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    status_path = tmp_path / "status.json"
    old = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=5)
    _write_status_json(
        status_path,
        {
            "state": "WATCHING",
            "ollama": "healthy",
            "gpu_temperature_c": 40,
            "restarts_10m": 0,
            "last_error": None,
            "updated_at": old.isoformat(),
        },
    )
    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(status_path))

    result = await session.call_tool("ollama_guard_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] == "stale"
    assert payload["age_seconds"] > node_observability_server.OLLAMA_GUARD_STALE_AFTER_SECONDS


@pytest.mark.anyio
async def test_ollama_guard_status_reports_fault_and_thermal_hold(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    status_path = tmp_path / "status.json"
    now = datetime.datetime.now(datetime.timezone.utc)
    _write_status_json(
        status_path,
        {
            "state": "FAULT",
            "ollama": "unhealthy",
            "gpu_temperature_c": 45,
            "restarts_10m": 3,
            "last_error": "raggiunto il limite di 3 restart in 600s",
            "updated_at": now.isoformat(),
        },
    )
    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(status_path))

    result = await session.call_tool("ollama_guard_status", arguments={})
    payload = json.loads(result.content[0].text)
    assert payload["status"] == "ok"
    assert payload["guard_state"] == "FAULT"
    assert payload["restarts_10m"] == 3
    assert "limite" in payload["last_error"]


@pytest.mark.anyio
async def test_ollama_guard_status_error_on_malformed_json(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    status_path = tmp_path / "status.json"
    status_path.write_text("{not valid json", encoding="utf-8")
    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(status_path))

    result = await session.call_tool("ollama_guard_status", arguments={})
    assert not result.is_error
    payload = json.loads(result.content[0].text)
    assert payload["status"] == "error"


@pytest.mark.anyio
async def test_ollama_guard_status_never_writes_the_file(
    session: ClientSession, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """Guardia esplicita: il tool deve SOLO leggere. Verifichiamo che il
    contenuto e il permesso del file non cambino mai dopo la chiamata.
    """
    status_path = tmp_path / "status.json"
    now = datetime.datetime.now(datetime.timezone.utc)
    original_payload = {
        "state": "WATCHING",
        "ollama": "healthy",
        "gpu_temperature_c": 39,
        "restarts_10m": 0,
        "last_error": None,
        "updated_at": now.isoformat(),
    }
    _write_status_json(status_path, original_payload)
    status_path.chmod(0o644)
    original_raw = status_path.read_text(encoding="utf-8")
    original_mode = status_path.stat().st_mode

    monkeypatch.setattr(node_observability_server, "OLLAMA_GUARD_STATUS_PATH", str(status_path))

    await session.call_tool("ollama_guard_status", arguments={})
    await session.call_tool("ollama_guard_status", arguments={})

    assert status_path.read_text(encoding="utf-8") == original_raw
    assert status_path.stat().st_mode == original_mode
