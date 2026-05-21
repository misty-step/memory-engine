#!/usr/bin/env python3
"""Append local Spellbook trace records for agent-assisted work."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SECRET_KEY_RE = re.compile(
    r"(token|secret|password|credential|private[_-]?key|api[_-]?key)",
    re.IGNORECASE,
)
SECRET_ASSIGNMENT_RE = re.compile(
    r"\b[A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|CREDENTIAL|PRIVATE_KEY|API_KEY)\b\s*=",
    re.IGNORECASE,
)
SECRET_VALUE_RES = [
    re.compile(r"\bsk-[A-Za-z0-9_-]{20,}\b"),
    re.compile(r"\bghp_[A-Za-z0-9_]{20,}\b"),
    re.compile(r"\bxox[abp]-[A-Za-z0-9-]{20,}\b"),
    re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    re.compile(r"\b\d{3}-\d{2}-\d{4}\b"),
]
CREDIT_CARD_CANDIDATE_RE = re.compile(r"\b(?:\d[ -]*?){13,19}\b")
REDUCTION_POLICY = (
    "refuse obvious secrets, credentials, SSNs, credit-card-like values, "
    "and raw private customer data; store refs instead of transcript bodies"
)


class TraceError(Exception):
    """Raised when a trace record cannot be safely written."""


def luhn_valid(value: str) -> bool:
    digits = [int(ch) for ch in re.sub(r"\D", "", value)]
    if len(digits) < 13 or len(digits) > 19:
        return False
    checksum = 0
    parity = len(digits) % 2
    for index, digit in enumerate(digits):
        if index % 2 == parity:
            digit *= 2
            if digit > 9:
                digit -= 9
        checksum += digit
    return checksum % 10 == 0


def run_git(repo: Path, args: list[str], default: str = "") -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        return default
    return result.stdout.strip()


def resolve_repo(path: str) -> Path:
    candidate = Path(path).expanduser().resolve()
    if not candidate.exists():
        raise TraceError(f"repo path does not exist: {candidate}")
    result = subprocess.run(
        ["git", "-C", str(candidate), "rev-parse", "--show-toplevel"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or "not a git repository"
        raise TraceError(f"unable to resolve repo root from {candidate}: {detail}")
    return Path(result.stdout.strip()).resolve()


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def split_values(values: list[str] | None) -> list[str]:
    if not values:
        return []
    out: list[str] = []
    for value in values:
        for part in value.split(","):
            clean = part.strip()
            if clean:
                out.append(clean)
    return out


def current_commits(repo: Path, limit: int) -> list[str]:
    text = run_git(repo, ["log", f"--max-count={limit}", "--format=%H"], "")
    return [line for line in text.splitlines() if line]


def trace_id(repo: Path, branch: str, backlog: str | None) -> str:
    seed = f"{repo}:{branch}:{backlog or 'no-backlog'}"
    return hashlib.sha256(seed.encode("utf-8")).hexdigest()[:16]


def check_safe(value: Any, path: str = "record") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if SECRET_KEY_RE.search(str(key)):
                raise TraceError(f"refusing secret-shaped field name at {path}.{key}")
            check_safe(child, f"{path}.{key}")
        return
    if isinstance(value, list):
        for idx, child in enumerate(value):
            check_safe(child, f"{path}[{idx}]")
        return
    if value is None:
        return
    text = str(value)
    if SECRET_ASSIGNMENT_RE.search(text):
        raise TraceError(f"refusing secret-shaped assignment at {path}")
    for regex in SECRET_VALUE_RES:
        if regex.search(text):
            raise TraceError(f"refusing secret- or private-data-shaped value at {path}")
    for match in CREDIT_CARD_CANDIDATE_RE.finditer(text):
        if luhn_valid(match.group(0)):
            raise TraceError(f"refusing secret- or private-data-shaped value at {path}")


def base_record(args: argparse.Namespace, kind: str) -> dict[str, Any]:
    repo = resolve_repo(args.repo)
    branch = args.branch or run_git(repo, ["branch", "--show-current"], "detached")
    head_sha = args.head_sha or run_git(repo, ["rev-parse", "HEAD"], "")
    record: dict[str, Any] = {
        "schema_version": 1,
        "ts": utc_now(),
        "trace_id": trace_id(repo, branch, args.backlog),
        "kind": kind,
        "repo": repo.name,
        "backlog_id": args.backlog,
        "branch": branch,
        "head_sha": head_sha,
        "commits": split_values(args.commit) or current_commits(repo, args.commit_limit),
        "evidence": split_values(args.evidence),
        "qa": split_values(getattr(args, "qa", None)),
        "demo": split_values(getattr(args, "demo", None)),
        "review": split_values(getattr(args, "review", None)),
        "transcript_refs": split_values(args.transcript_ref),
        "note": args.note,
        "redaction_policy": REDUCTION_POLICY,
    }
    return {key: value for key, value in record.items() if value not in (None, [], "")}


def append_record(repo: Path, record: dict[str, Any]) -> Path:
    check_safe(record)
    trace_dir = repo / ".spellbook" / "traces"
    trace_dir.mkdir(parents=True, exist_ok=True)
    path = trace_dir / "traces.jsonl"
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
    return path


def handle_append(args: argparse.Namespace) -> dict[str, Any]:
    record = base_record(args, args.kind)
    record["record_path"] = ".spellbook/traces/traces.jsonl"
    repo = resolve_repo(args.repo)
    append_record(repo, record)
    return record


def handle_final(args: argparse.Namespace) -> dict[str, Any]:
    if not args.transcript_ref and not args.no_transcript_reason:
        raise TraceError("trace.final requires --transcript-ref or --no-transcript-reason")
    if not args.merged_sha:
        raise TraceError("trace.final requires --merged-sha")
    record = base_record(args, "trace.final")
    record["merged_sha"] = args.merged_sha
    record["no_transcript_reason"] = args.no_transcript_reason
    record["record_path"] = ".spellbook/traces/traces.jsonl"
    repo = resolve_repo(args.repo)
    append_record(repo, record)
    return {key: value for key, value in record.items() if value not in (None, [], "")}


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--repo", default=".", help="Path inside target git repository.")
    parser.add_argument("--backlog", help="Backlog/spec ID, for example 056.")
    parser.add_argument("--branch", help="Override detected branch.")
    parser.add_argument("--head-sha", help="Override detected HEAD sha.")
    parser.add_argument("--commit", action="append", help="Commit sha; may repeat or comma-separate.")
    parser.add_argument("--commit-limit", type=int, default=20, help="Fallback recent commit count.")
    parser.add_argument("--evidence", action="append", help="Evidence path/ref; may repeat or comma-separate.")
    parser.add_argument("--transcript-ref", action="append", help="External transcript/session ref.")
    parser.add_argument("--note", help="Short metadata note. Do not paste raw transcripts.")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Append Spellbook trace JSONL records.")
    sub = parser.add_subparsers(dest="command", required=True)

    append = sub.add_parser("append", help="Append a non-final trace record.")
    add_common(append)
    append.add_argument("--kind", default="trace.note", help="Record kind, default trace.note.")
    append.set_defaults(func=handle_append)

    final = sub.add_parser("final", help="Append the final work record for /ship.")
    add_common(final)
    final.add_argument("--merged-sha", help="Landed commit sha.")
    final.add_argument("--qa", action="append", help="QA evidence path/ref.")
    final.add_argument("--demo", action="append", help="Demo evidence path/ref.")
    final.add_argument("--review", action="append", help="Review verdict path/ref.")
    final.add_argument("--no-transcript-reason", help="Required when no transcript ref exists.")
    final.set_defaults(func=handle_final)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        record = args.func(args)
    except TraceError as exc:
        print(f"trace: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
