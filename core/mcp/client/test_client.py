"""Client MCP di test per il server `node_observability_server`.

Si connette via Streamable HTTP, esegue initialize, elenca i tool
disponibili, chiama i 3 tool esposti e stampa il risultato.

Usa l'SDK ufficiale MCP Python v2 (`mcp.client.streamable_http`,
`mcp.ClientSession`), coerente col server in server/node_observability_server.py.

Esegui con il server già avviato:
    python core/mcp/client/test_client.py [http://127.0.0.1:8765/mcp]
"""

from __future__ import annotations

import asyncio
import sys

from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

DEFAULT_URL = "http://127.0.0.1:8811/mcp"

EXPECTED_TOOLS = {"node_status", "ollama_status", "gpu_status"}


async def run(url: str) -> None:
    print(f"Connessione al server MCP: {url}")
    async with streamable_http_client(url) as (read_stream, write_stream):
        async with ClientSession(read_stream, write_stream) as session:
            init_result = await session.initialize()
            print(
                f"initialize OK: server={init_result.server_info.name} "
                f"protocolVersion={init_result.protocol_version}"
            )

            tools_result = await session.list_tools()
            tool_names = {t.name for t in tools_result.tools}
            print(f"tools/list -> {sorted(tool_names)}")

            missing = EXPECTED_TOOLS - tool_names
            unexpected = tool_names - EXPECTED_TOOLS
            if missing:
                raise AssertionError(f"tool attesi mancanti: {sorted(missing)}")
            if unexpected:
                raise AssertionError(
                    f"trovati tool NON previsti (violazione policy read-only): {sorted(unexpected)}"
                )

            for tool_name in sorted(EXPECTED_TOOLS):
                print(f"\n--- chiamata tool: {tool_name} ---")
                result = await session.call_tool(tool_name, arguments={})
                if result.is_error:
                    print(f"  ERRORE riportato dal tool: {result.content}")
                else:
                    for block in result.content:
                        if hasattr(block, "text"):
                            print(f"  {block.text}")
                        else:
                            print(f"  {block}")

    print("\nTutti i tool sono stati chiamati con successo.")


def main() -> None:
    url = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_URL
    asyncio.run(run(url))


if __name__ == "__main__":
    main()
