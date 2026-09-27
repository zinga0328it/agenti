"""Configurazione pytest per i test del server MCP.

Il plugin pytest di `anyio` (bundlato con l'SDK MCP) richiede una fixture
`anyio_backend`: fissiamo asyncio come unico backend usato nei test, che è
anche il runtime realmente usato dal server e dal client in produzione.
"""
import pytest


@pytest.fixture
def anyio_backend():
    return "asyncio"
