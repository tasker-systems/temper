"""A real HTTP server on localhost, rather than a mocked transport.

The gem stubs with WebMock. The equivalent here would be monkeypatching urllib3's
`PoolManager.request`, which is precisely the layer the client's own configuration
touches — `retries=False`, the pool, the form encoder — so a stub there proves the
skin calls a function, not that the request leaves the process in the right shape.
A `ThreadingHTTPServer` costs one fixture and asserts on the bytes on the wire.
"""

from __future__ import annotations

import json
import threading
from collections import deque
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

import pytest


@dataclass
class RecordedRequest:
    method: str
    path: str
    headers: dict[str, str]
    body: bytes

    @property
    def text(self) -> str:
        return self.body.decode("utf-8")

    def form(self) -> dict[str, str]:
        """Parse the body the way the SERVER does, not the way the client wrote it."""
        from urllib.parse import parse_qsl

        return dict(parse_qsl(self.text, strict_parsing=bool(self.text)))

    def json(self) -> Any:
        return json.loads(self.text)


@dataclass
class ScriptedResponse:
    status: int = 200
    body: bytes = b""
    headers: dict[str, str] = field(default_factory=dict)


class RecordingServer:
    def __init__(self) -> None:
        self.requests: list[RecordedRequest] = []
        self._queue: deque[ScriptedResponse] = deque()
        self._default: ScriptedResponse | None = None
        self._lock = threading.Lock()
        self._httpd: ThreadingHTTPServer | None = None

    # -- scripting -----------------------------------------------------------

    def respond(
        self,
        *,
        status: int = 200,
        body: Any = b"",
        headers: dict[str, str] | None = None,
        times: int = 1,
    ) -> RecordingServer:
        """Queue `times` copies of one response. Responses are served in order."""
        payload = self._encode(body)
        for _ in range(times):
            self._queue.append(ScriptedResponse(status, payload, dict(headers or {})))
        return self

    def respond_json(self, obj: Any, *, status: int = 200, times: int = 1) -> RecordingServer:
        return self.respond(
            status=status,
            body=json.dumps(obj),
            headers={"Content-Type": "application/json"},
            times=times,
        )

    def always(
        self,
        *,
        status: int = 200,
        body: Any = b"",
        headers: dict[str, str] | None = None,
    ) -> None:
        self._default = ScriptedResponse(status, self._encode(body), dict(headers or {}))

    @staticmethod
    def _encode(body: Any) -> bytes:
        if isinstance(body, bytes):
            return body
        if isinstance(body, str):
            return body.encode("utf-8")
        return json.dumps(body).encode("utf-8")

    # -- serving -------------------------------------------------------------

    @property
    def url(self) -> str:
        assert self._httpd is not None
        _host, port = self._httpd.server_address[:2]
        return f"http://127.0.0.1:{port}"

    def _next(self) -> ScriptedResponse:
        with self._lock:
            if self._queue:
                return self._queue.popleft()
            if self._default is not None:
                return self._default
        return ScriptedResponse(status=599, body=b'{"error":{"message":"no scripted response"}}')

    def _record(self, request: RecordedRequest) -> None:
        with self._lock:
            self.requests.append(request)


@pytest.fixture
def server() -> Iterator[RecordingServer]:
    recorder = RecordingServer()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def _handle(self) -> None:
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else b""
            recorder._record(
                RecordedRequest(
                    method=self.command,
                    path=self.path,
                    headers={k.lower(): v for k, v in self.headers.items()},
                    body=body,
                )
            )
            scripted = recorder._next()
            self.send_response(scripted.status)
            for name, value in scripted.headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(scripted.body)))
            self.end_headers()
            self.wfile.write(scripted.body)

        do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = _handle

        def log_message(self, *args: Any) -> None:  # keep pytest output readable
            return

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    recorder._httpd = httpd
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield recorder
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)


@pytest.fixture(autouse=True)
def _isolated_connection() -> Iterator[None]:
    """No test may inherit another's process-global connection."""
    import temper
    import temper.connection

    yield
    temper.reset_connection()
    # Reach for the MODULE, never `from temper import connection` — see the note on
    # `current_connection`.
    temper.connection._default = None
