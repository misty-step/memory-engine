#!/usr/bin/env python3
"""Trusted target-blind OpenRouter proxy and provider-call attestor.

The target receives only a one-run proxy capability and a Unix socket. This
process owns the real scoped key, the upstream connection, and the attestation
path; target output and stdout are never provider evidence.

Portability contract: the proxy must run on the oldest declared host
interpreter (python3 == 3.7), so it avoids the 3.8+ ``Path.unlink`` keyword
and 3.10+ socket-timeout aliasing.

Egress contract: upstream requests go through a redirect-refusing opener so
the Authorization header can never follow a Location to another origin; the
request payload must match the exact benchmark shape (allowlisted keys, one
allowlisted model, bounded messages); and forwarded traffic is bounded by
global call, byte, and concurrency budgets. Refused traffic is answered
locally and never recorded as provider proof.
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
UPSTREAM_TIMEOUT_SECONDS = 120
MAX_REQUEST_BYTES = 16 * 1024 * 1024
SOCKET_READ_TIMEOUT_SECONDS = 30
DEFAULT_ALLOWED_MODEL = "google/gemini-3.5-flash"
DEFAULT_MAX_CALLS = 128
DEFAULT_MAX_TOTAL_BYTES = 256 * 1024 * 1024
DEFAULT_MAX_CONCURRENCY = 4
MAX_MESSAGES = 8
MAX_MESSAGE_CONTENT_BYTES = 256 * 1024
MAX_SCHEMA_NAME_BYTES = 128
ALLOWED_ROLES = ("system", "user")
ALLOWED_PAYLOAD_KEYS = frozenset(
    ("model", "messages", "response_format", "provider", "usage")
)
REQUIRED_PROVIDER_OPTIONS = {"require_parameters": True, "allow_fallbacks": True}
REQUIRED_USAGE_OPTIONS = {"include": True}


def remove_file_if_present(path: Path) -> None:
    """The forgiving ``Path.unlink`` keyword requires 3.8; stay 3.7-portable."""
    try:
        os.unlink(str(path))
    except FileNotFoundError:
        pass


class RefuseRedirects(urllib.request.HTTPRedirectHandler):
    """Refuse every upstream redirect outright.

    ``urlopen`` follows redirects while preserving non-content headers, which
    would resend the provider bearer key to an attacker-chosen Location.
    Returning ``None`` from ``redirect_request`` makes urllib raise the 3xx
    as an ``HTTPError`` instead of following it.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D102
        return None


UPSTREAM_OPENER = urllib.request.build_opener(RefuseRedirects())


def validate_payload(payload: object, allowed_model: str):
    """Return an error string, or None when the payload matches the exact
    benchmark request shape. Anything else is refused before egress."""
    if not isinstance(payload, dict):
        return "payload must be a JSON object"
    unexpected = set(payload) - ALLOWED_PAYLOAD_KEYS
    if unexpected:
        return "payload keys are not allowlisted: " + ",".join(sorted(unexpected))
    if payload.get("model") != allowed_model:
        return "model is not allowlisted"
    messages = payload.get("messages")
    if not isinstance(messages, list) or not 1 <= len(messages) <= MAX_MESSAGES:
        return "messages must be a nonempty bounded list"
    for message in messages:
        if not isinstance(message, dict) or set(message) != {"role", "content"}:
            return "messages must contain exactly role and content"
        if message["role"] not in ALLOWED_ROLES:
            return "message role is not allowlisted"
        content = message["content"]
        if not isinstance(content, str):
            return "message content must be a string"
        if len(content.encode("utf-8")) > MAX_MESSAGE_CONTENT_BYTES:
            return "message content exceeds the per-message byte bound"
    if "provider" in payload and payload["provider"] != REQUIRED_PROVIDER_OPTIONS:
        return "provider options are not allowlisted"
    if "usage" in payload and payload["usage"] != REQUIRED_USAGE_OPTIONS:
        return "usage options are not allowlisted"
    if "response_format" in payload:
        response_format = payload["response_format"]
        if not isinstance(response_format, dict) or set(response_format) != {
            "type",
            "json_schema",
        }:
            return "response_format must contain exactly type and json_schema"
        if response_format["type"] != "json_schema":
            return "response_format type is not allowlisted"
        json_schema = response_format["json_schema"]
        if not isinstance(json_schema, dict) or set(json_schema) != {
            "name",
            "strict",
            "schema",
        }:
            return "json_schema must contain exactly name, strict, and schema"
        if not isinstance(json_schema["name"], str) or (
            len(json_schema["name"].encode("utf-8")) > MAX_SCHEMA_NAME_BYTES
        ):
            return "json_schema name must be a short string"
        if json_schema["strict"] is not True:
            return "json_schema must be strict"
        if not isinstance(json_schema["schema"], dict):
            return "json_schema schema must be an object"
    return None


class State:
    def __init__(
        self,
        target_sha: str,
        token: str,
        provider_key: bytes,
        allowed_model: str,
        max_calls: int,
        max_total_bytes: int,
        max_concurrency: int,
    ) -> None:
        self.target_sha = target_sha
        self.token = token
        self.provider_key = provider_key
        self.allowed_model = allowed_model
        self.max_calls = max_calls
        self.max_total_bytes = max_total_bytes
        self.calls = []
        self.forwarded_calls = 0
        self.rejected_calls = 0
        self.total_bytes = 0
        self.lock = threading.Lock()
        self.concurrency = threading.BoundedSemaphore(max(1, max_concurrency))

    def reserve(self, request_bytes: int) -> bool:
        """Admit one upstream call inside the global budgets, atomically."""
        with self.lock:
            if self.forwarded_calls >= self.max_calls:
                self.rejected_calls += 1
                return False
            if self.total_bytes + request_bytes > self.max_total_bytes:
                self.rejected_calls += 1
                return False
            self.forwarded_calls += 1
            self.total_bytes += request_bytes
            return True

    def reject(self) -> None:
        with self.lock:
            self.rejected_calls += 1

    def record(self, request_body: bytes, response_body: bytes, status: int) -> None:
        with self.lock:
            self.total_bytes += len(response_body)
            self.calls.append(
                {
                    "request_sha256": hashlib.sha256(request_body).hexdigest(),
                    "response_sha256": hashlib.sha256(response_body).hexdigest(),
                    "http_status": status,
                    "successful": 200 <= status < 300,
                }
            )

    def snapshot(self):
        with self.lock:
            return list(self.calls), self.rejected_calls, self.total_bytes


class ProxyHandler(socketserver.StreamRequestHandler):
    server = None  # type: ProxyServer

    def setup(self) -> None:
        super().setup()
        self.connection.settimeout(SOCKET_READ_TIMEOUT_SECONDS)

    def handle(self) -> None:
        try:
            line = self.rfile.readline(MAX_REQUEST_BYTES + 1)
        except OSError:
            # socket.timeout is an OSError on every supported interpreter;
            # 3.7 does not alias it to TimeoutError.
            return
        if len(line) > MAX_REQUEST_BYTES:
            self._reply(413, "request too large")
            return
        try:
            request = json.loads(line.decode("utf-8"))
            token = request["token"]
            payload = request["payload"]
        except (UnicodeDecodeError, json.JSONDecodeError, KeyError, TypeError):
            self._reply(400, "malformed proxy request")
            return
        if token != self.server.state.token:
            self._reply(401, "invalid proxy capability")
            return
        schema_error = validate_payload(payload, self.server.state.allowed_model)
        if schema_error is not None:
            self.server.state.reject()
            self._reply(400, "request is not allowlisted: " + schema_error)
            return
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        if not self.server.state.reserve(len(body)):
            self._reply(429, "trusted provider proxy budget exhausted")
            return

        upstream = urllib.request.Request(
            UPSTREAM_URL,
            data=body,
            headers={
                "Authorization": "Bearer "
                + self.server.state.provider_key.decode("utf-8"),
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with self.server.state.concurrency:
            try:
                with UPSTREAM_OPENER.open(
                    upstream, timeout=UPSTREAM_TIMEOUT_SECONDS
                ) as response:
                    response_body = response.read(MAX_REQUEST_BYTES)
                    status = response.getcode()
            except urllib.error.HTTPError as error:
                # A refused redirect surfaces here as its 3xx status and is
                # recorded as an unsuccessful call; the key was not resent.
                response_body = error.read(MAX_REQUEST_BYTES)
                status = error.code
            except (OSError, urllib.error.URLError):
                response_body = (
                    b'{"error":{"message":"trusted provider proxy transport failure"}}'
                )
                status = 502

        redacted_body = response_body.replace(
            self.server.state.provider_key, b"[redacted]"
        )
        self.server.state.record(body, redacted_body, status)
        self._reply(status, redacted_body.decode("utf-8", errors="replace"))

    def _reply(self, status: int, body: str) -> None:
        try:
            self.wfile.write(
                json.dumps(
                    {"status": status, "body": body}, separators=(",", ":")
                ).encode("utf-8")
                + b"\n"
            )
        except OSError:
            pass


class ProxyServer(socketserver.ThreadingUnixStreamServer):
    daemon_threads = True

    def __init__(self, socket_path: str, state: State) -> None:
        super().__init__(socket_path, ProxyHandler)
        self.state = state


def write_attestation(path: Path, state: State) -> None:
    calls, rejected, total_bytes = state.snapshot()
    successful = sum(1 for call in calls if call["successful"])
    attestation = {
        "schema": "memory-engine/generation-061-provider-attestation/v1",
        "target_sha": state.target_sha,
        "provider_calls": len(calls),
        "successful_provider_calls": successful,
        "rejected_provider_calls": rejected,
        "total_provider_bytes": total_bytes,
        "canonical_acceptance": (
            "provider-calls-observed"
            if len(calls) >= 15 and successful == len(calls)
            else "rejected"
        ),
        "calls": calls,
    }
    temporary = path.with_name(".{}.tmp-{}".format(path.name, os.getpid()))
    temporary.write_text(json.dumps(attestation, sort_keys=True, indent=1) + "\n")
    os.chmod(str(temporary), 0o600)
    os.replace(str(temporary), str(path))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", type=Path, required=True)
    parser.add_argument("--attestation", type=Path, required=True)
    parser.add_argument("--target-sha", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--provider-key-fd", type=int, default=3)
    parser.add_argument("--model", default=DEFAULT_ALLOWED_MODEL)
    parser.add_argument("--max-calls", type=int, default=DEFAULT_MAX_CALLS)
    parser.add_argument(
        "--max-total-bytes", type=int, default=DEFAULT_MAX_TOTAL_BYTES
    )
    parser.add_argument(
        "--max-concurrency", type=int, default=DEFAULT_MAX_CONCURRENCY
    )
    args = parser.parse_args()
    provider_key = os.read(args.provider_key_fd, 4096).strip()
    if not provider_key:
        raise SystemExit(
            "trusted provider proxy requires its key on a private file descriptor"
        )
    remove_file_if_present(args.socket)

    state = State(
        args.target_sha,
        args.token,
        provider_key,
        args.model,
        args.max_calls,
        args.max_total_bytes,
        args.max_concurrency,
    )
    server = ProxyServer(str(args.socket), state)
    os.chmod(str(args.socket), 0o600)

    def stop(_signum, _frame) -> None:
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        # A short poll interval keeps SIGTERM-to-exit deterministic and
        # bounded; handler threads are daemons and cannot block shutdown.
        server.serve_forever(poll_interval=0.1)
    finally:
        server.server_close()
        remove_file_if_present(args.socket)
        write_attestation(args.attestation, state)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
