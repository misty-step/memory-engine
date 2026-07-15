#!/usr/bin/env python3
"""Trusted target-blind OpenRouter proxy and provider-call attestor.

The target receives only a one-run proxy capability and a Unix socket. This
process owns the real scoped key, the upstream connection, and the attestation
path; target output and stdout are never provider evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import socketserver
import threading
import urllib.error
import urllib.request
from pathlib import Path

UPSTREAM_URL = "https://openrouter.ai/api/v1/chat/completions"
MAX_REQUEST_BYTES = 16 * 1024 * 1024
SOCKET_READ_TIMEOUT_SECONDS = 30


class State:
    def __init__(self, target_sha: str, token: str, provider_key: bytes) -> None:
        self.target_sha = target_sha
        self.token = token
        self.provider_key = provider_key
        self.calls: list[dict[str, object]] = []
        self.lock = threading.Lock()

    def record(self, request_body: bytes, response_body: bytes, status: int) -> None:
        with self.lock:
            self.calls.append(
                {
                    "request_sha256": hashlib.sha256(request_body).hexdigest(),
                    "response_sha256": hashlib.sha256(response_body).hexdigest(),
                    "http_status": status,
                    "successful": 200 <= status < 300,
                }
            )

    def snapshot(self) -> list[dict[str, object]]:
        with self.lock:
            return list(self.calls)


class ProxyHandler(socketserver.StreamRequestHandler):
    server: "ProxyServer"

    def setup(self) -> None:
        super().setup()
        self.connection.settimeout(SOCKET_READ_TIMEOUT_SECONDS)

    def handle(self) -> None:
        try:
            line = self.rfile.readline(MAX_REQUEST_BYTES + 1)
        except TimeoutError:
            return
        if len(line) > MAX_REQUEST_BYTES:
            self._reply(413, "request too large")
            return
        try:
            request = json.loads(line)
            token = request["token"]
            body = json.dumps(request["payload"], separators=(",", ":")).encode()
        except (json.JSONDecodeError, KeyError, TypeError):
            self._reply(400, "malformed proxy request")
            return
        if token != self.server.state.token:
            self._reply(401, "invalid proxy capability")
            return

        upstream = urllib.request.Request(
            UPSTREAM_URL,
            data=body,
            headers={
                "Authorization": f"Bearer {self.server.state.provider_key.decode()}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(upstream, timeout=120) as response:
                response_body = response.read(MAX_REQUEST_BYTES)
                status = response.status
        except urllib.error.HTTPError as error:
            response_body = error.read(MAX_REQUEST_BYTES)
            status = error.code
        except (OSError, urllib.error.URLError):
            response_body = b'{"error":{"message":"trusted provider proxy transport failure"}}'
            status = 502

        redacted_body = response_body.replace(
            self.server.state.provider_key, b"[redacted]"
        )
        self.server.state.record(body, redacted_body, status)
        self._reply(status, redacted_body.decode("utf-8", errors="replace"))

    def _reply(self, status: int, body: str) -> None:
        self.wfile.write(
            json.dumps({"status": status, "body": body}, separators=(",", ":")).encode()
            + b"\n"
        )


class ProxyServer(socketserver.ThreadingUnixStreamServer):
    daemon_threads = True

    def __init__(self, socket_path: str, state: State) -> None:
        super().__init__(socket_path, ProxyHandler)
        self.state = state


def write_attestation(path: Path, state: State) -> None:
    calls = state.snapshot()
    successful = sum(1 for call in calls if call["successful"])
    attestation = {
        "schema": "memory-engine/generation-061-provider-attestation/v1",
        "target_sha": state.target_sha,
        "provider_calls": len(calls),
        "successful_provider_calls": successful,
        "canonical_acceptance": (
            "provider-calls-observed"
            if len(calls) >= 15 and successful == len(calls)
            else "rejected"
        ),
        "calls": calls,
    }
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(attestation, sort_keys=True) + "\n")
    os.chmod(temporary, 0o600)
    os.replace(temporary, path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", type=Path, required=True)
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument("--target-sha", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--provider-key-fd", type=int, default=3)
    args = parser.parse_args()
    provider_key = os.read(args.provider_key_fd, 4096).strip()
    if not provider_key:
        raise SystemExit("trusted provider proxy requires its key on a private file descriptor")
    args.socket.unlink(missing_ok=True)

    state = State(args.target_sha, args.token, provider_key)
    server = ProxyServer(str(args.socket), state)
    os.chmod(args.socket, 0o600)

    def stop(_signum: int, _frame: object) -> None:
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        server.serve_forever()
    finally:
        server.server_close()
        args.socket.unlink(missing_ok=True)
        write_attestation(args.attestation, state)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
