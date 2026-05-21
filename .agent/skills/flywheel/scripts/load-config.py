#!/usr/bin/env python3
"""Load and validate repo-local .spellbook config for installed skills."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

VALID_CONFIGS = ("deploy", "monitor", "flywheel")
DURATION_FIELDS = {
    "deploy": ("rollback_grace",),
    "monitor": ("grace_window", "poll_interval"),
    "flywheel": ("cadence",),
}
DURATION_RE = re.compile(r"^\s*(\d+)\s*([smhd])\s*$")
DURATION_SECONDS = {"s": 1, "m": 60, "h": 3600, "d": 86400}

DEPLOY_KEYS = {
    "schema_version",
    "target",
    "app",
    "env",
    "healthcheck_url",
    "deploy_cmd",
    "rollback_cmd",
    "pre_deploy_cmd",
    "post_deploy_cmd",
    "rollback_grace",
    "require_ci_green",
    "idempotent",
}
DEPLOY_TARGETS = {"fly", "vercel", "cloudflare", "aws", "s3", "docker", "k8s", "custom"}
MONITOR_KEYS = {"schema_version", "grace_window", "poll_interval", "healthcheck", "signals"}
HEALTHCHECK_KEYS = {"url", "expected_status", "hard_fail_on_5xx"}
SIGNAL_KEYS = {"name", "source", "query", "url", "threshold", "jq", "hard_fail"}
SIGNAL_SOURCES = {"datadog", "prometheus", "grafana", "loki", "logs", "custom"}
FLYWHEEL_KEYS = {
    "schema_version",
    "cadence",
    "max_cycles",
    "budget_tokens",
    "backlog_includes",
    "stop_on_monitor_alert",
    "stop_on_phase_failed",
    "stop_on_budget_exhausted",
}


class ConfigError(Exception):
    """Raised when config input is invalid or cannot be loaded."""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Load and validate .spellbook/<name>.yaml and print normalized JSON."
        )
    )
    parser.add_argument("name", choices=VALID_CONFIGS, help="Config name to load.")
    parser.add_argument(
        "--repo",
        default=".",
        help="Path inside target repository (git root resolved from this path).",
    )
    parser.add_argument(
        "--config",
        help="Explicit config path (defaults to <repo>/.spellbook/<name>.yaml).",
    )
    parser.add_argument(
        "--optional",
        action="store_true",
        help="Return {} and exit 0 when config file is missing.",
    )
    return parser.parse_args()


def resolve_repo_root(repo_path: str) -> Path:
    candidate = Path(repo_path).expanduser().resolve()
    if not candidate.exists():
        raise ConfigError(f"--repo path does not exist: {candidate}")
    result = subprocess.run(
        ["git", "-C", str(candidate), "rev-parse", "--show-toplevel"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        stderr = result.stderr.strip() or "not a git repository"
        raise ConfigError(
            f"unable to resolve repo root from --repo {candidate}: {stderr}"
        )
    return Path(result.stdout.strip()).resolve()


def resolve_config_path(name: str, repo_root: Path, config_path: str | None) -> Path:
    if config_path:
        return Path(config_path).expanduser().resolve()
    return repo_root / ".spellbook" / f"{name}.yaml"


def strip_comment(line: str) -> str:
    quote: str | None = None
    for idx, char in enumerate(line):
        if char in ("'", '"'):
            quote = None if quote == char else char if quote is None else quote
        if char == "#" and quote is None:
            return line[:idx]
    return line


def parse_scalar(raw: str) -> Any:
    value = raw.strip()
    if value in ("true", "false"):
        return value == "true"
    if re.fullmatch(r"-?\d+", value):
        return int(value)
    if (
        len(value) >= 2
        and value[0] == value[-1]
        and value[0] in ("'", '"')
    ):
        return value[1:-1]
    return value


def split_key_value(path: Path, lineno: int, text: str) -> tuple[str, str]:
    if ":" not in text:
        raise ConfigError(f"{path}:{lineno}: expected 'key: value'")
    key, value = text.split(":", 1)
    key = key.strip()
    if not key:
        raise ConfigError(f"{path}:{lineno}: empty key")
    return key, value.strip()


def load_simple_yaml(path: Path) -> dict[str, Any]:
    try:
        raw_lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise ConfigError(f"{path}: unable to read file: {exc}") from exc

    data: dict[str, Any] = {}
    idx = 0
    while idx < len(raw_lines):
        lineno = idx + 1
        raw = strip_comment(raw_lines[idx]).rstrip()
        idx += 1
        if not raw.strip():
            continue
        indent = len(raw) - len(raw.lstrip(" "))
        if indent != 0:
            raise ConfigError(f"{path}:{lineno}: expected top-level key")
        key, value = split_key_value(path, lineno, raw)
        if value:
            data[key] = parse_scalar(value)
            continue
        if key == "healthcheck":
            child: dict[str, Any] = {}
            while idx < len(raw_lines):
                child_lineno = idx + 1
                child_raw = strip_comment(raw_lines[idx]).rstrip()
                if not child_raw.strip():
                    idx += 1
                    continue
                child_indent = len(child_raw) - len(child_raw.lstrip(" "))
                if child_indent == 0:
                    break
                if child_indent != 2:
                    raise ConfigError(f"{path}:{child_lineno}: expected two-space indent")
                child_key, child_value = split_key_value(path, child_lineno, child_raw)
                if not child_value:
                    raise ConfigError(f"{path}:{child_lineno}: expected scalar value")
                child[child_key] = parse_scalar(child_value)
                idx += 1
            data[key] = child
            continue
        if key in ("signals", "backlog_includes"):
            items: list[Any] = []
            current: dict[str, Any] | None = None
            while idx < len(raw_lines):
                item_lineno = idx + 1
                item_raw = strip_comment(raw_lines[idx]).rstrip()
                if not item_raw.strip():
                    idx += 1
                    continue
                item_indent = len(item_raw) - len(item_raw.lstrip(" "))
                if item_indent == 0:
                    break
                if key == "backlog_includes":
                    if item_indent != 2 or not item_raw.lstrip().startswith("- "):
                        raise ConfigError(f"{path}:{item_lineno}: expected list item")
                    items.append(parse_scalar(item_raw.lstrip()[2:]))
                    idx += 1
                    continue
                if item_indent == 2 and item_raw.lstrip().startswith("- "):
                    current = {}
                    items.append(current)
                    rest = item_raw.lstrip()[2:].strip()
                    if rest:
                        child_key, child_value = split_key_value(path, item_lineno, rest)
                        current[child_key] = parse_scalar(child_value)
                    idx += 1
                    continue
                if item_indent == 4 and current is not None:
                    child_key, child_value = split_key_value(path, item_lineno, item_raw)
                    if not child_value:
                        raise ConfigError(f"{path}:{item_lineno}: expected scalar value")
                    current[child_key] = parse_scalar(child_value)
                    idx += 1
                    continue
                raise ConfigError(f"{path}:{item_lineno}: expected signal list item")
            data[key] = items
            continue
        raise ConfigError(f"{path}:{lineno}: nested key '{key}' is not supported")
    return data


def fail_unknown(config_path: Path, data: dict[str, Any], allowed: set[str], location: str) -> None:
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise ConfigError(f"{config_path}: unknown key at {location}: {', '.join(unknown)}")


def require(config_path: Path, data: dict[str, Any], keys: tuple[str, ...], location: str) -> None:
    for key in keys:
        if key not in data:
            raise ConfigError(f"{config_path}: missing required key at {location}: {key}")


def require_schema_version(config_path: Path, data: dict[str, Any]) -> None:
    require(config_path, data, ("schema_version",), "<root>")
    if data["schema_version"] != 1:
        raise ConfigError(f"{config_path}: schema violation at schema_version: expected 1")


def require_string(config_path: Path, data: dict[str, Any], key: str, location: str) -> None:
    if key in data and (not isinstance(data[key], str) or not data[key]):
        raise ConfigError(f"{config_path}: schema violation at {location}.{key}: expected non-empty string")


def require_bool(config_path: Path, data: dict[str, Any], key: str, location: str) -> None:
    if key in data and not isinstance(data[key], bool):
        raise ConfigError(f"{config_path}: schema violation at {location}.{key}: expected boolean")


def require_int_min(config_path: Path, data: dict[str, Any], key: str, minimum: int, location: str) -> None:
    if key in data and (not isinstance(data[key], int) or data[key] < minimum):
        raise ConfigError(f"{config_path}: schema violation at {location}.{key}: expected integer >= {minimum}")


def require_url(config_path: Path, data: dict[str, Any], key: str, location: str) -> None:
    require_string(config_path, data, key, location)
    if key in data and not re.match(r"^https?://", data[key]):
        raise ConfigError(f"{config_path}: schema violation at {location}.{key}: expected http(s) URL")


def validate_deploy(config_path: Path, data: dict[str, Any]) -> None:
    fail_unknown(config_path, data, DEPLOY_KEYS, "<root>")
    require_schema_version(config_path, data)
    require(config_path, data, ("target", "app"), "<root>")
    for key in DEPLOY_KEYS - {"schema_version", "require_ci_green", "idempotent"}:
        require_string(config_path, data, key, "<root>")
    for key in ("require_ci_green", "idempotent"):
        require_bool(config_path, data, key, "<root>")
    require_url(config_path, data, "healthcheck_url", "<root>")
    if data["target"] not in DEPLOY_TARGETS:
        raise ConfigError(f"{config_path}: schema violation at target: unsupported target {data['target']}")
    if data["target"] == "custom":
        require(config_path, data, ("deploy_cmd", "rollback_cmd"), "<root>")


def validate_monitor(config_path: Path, data: dict[str, Any]) -> None:
    fail_unknown(config_path, data, MONITOR_KEYS, "<root>")
    require_schema_version(config_path, data)
    if "healthcheck" not in data and "signals" not in data:
        raise ConfigError(f"{config_path}: missing required key at <root>: healthcheck or signals")
    for key in ("grace_window", "poll_interval"):
        require_string(config_path, data, key, "<root>")
    healthcheck = data.get("healthcheck")
    if healthcheck is not None:
        if not isinstance(healthcheck, dict):
            raise ConfigError(f"{config_path}: schema violation at healthcheck: expected object")
        fail_unknown(config_path, healthcheck, HEALTHCHECK_KEYS, "healthcheck")
        require(config_path, healthcheck, ("url",), "healthcheck")
        require_url(config_path, healthcheck, "url", "healthcheck")
        require_int_min(config_path, healthcheck, "expected_status", 100, "healthcheck")
        if "expected_status" in healthcheck and healthcheck["expected_status"] > 599:
            raise ConfigError(f"{config_path}: schema violation at healthcheck.expected_status: expected <= 599")
        require_bool(config_path, healthcheck, "hard_fail_on_5xx", "healthcheck")
    signals = data.get("signals")
    if signals is not None:
        if not isinstance(signals, list) or not signals:
            raise ConfigError(f"{config_path}: schema violation at signals: expected non-empty list")
        for index, signal in enumerate(signals):
            location = f"signals.{index}"
            if not isinstance(signal, dict):
                raise ConfigError(f"{config_path}: schema violation at {location}: expected object")
            fail_unknown(config_path, signal, SIGNAL_KEYS, location)
            require(config_path, signal, ("name", "source", "threshold"), location)
            if "query" not in signal and "url" not in signal:
                raise ConfigError(f"{config_path}: missing required key at {location}: query or url")
            for key in SIGNAL_KEYS - {"hard_fail"}:
                require_string(config_path, signal, key, location)
            require_bool(config_path, signal, "hard_fail", location)
            require_url(config_path, signal, "url", location)
            if signal["source"] not in SIGNAL_SOURCES:
                raise ConfigError(f"{config_path}: schema violation at {location}.source: unsupported source {signal['source']}")


def validate_flywheel(config_path: Path, data: dict[str, Any]) -> None:
    fail_unknown(config_path, data, FLYWHEEL_KEYS, "<root>")
    require_schema_version(config_path, data)
    require_string(config_path, data, "cadence", "<root>")
    for key in ("max_cycles", "budget_tokens"):
        require_int_min(config_path, data, key, 1, "<root>")
    for key in ("stop_on_monitor_alert", "stop_on_phase_failed", "stop_on_budget_exhausted"):
        require_bool(config_path, data, key, "<root>")
    includes = data.get("backlog_includes")
    if includes is not None:
        if not isinstance(includes, list) or not includes:
            raise ConfigError(f"{config_path}: schema violation at backlog_includes: expected non-empty list")
        for item in includes:
            if not isinstance(item, str) or not item:
                raise ConfigError(f"{config_path}: schema violation at backlog_includes: expected string items")


def validate_config(config_name: str, config_path: Path, data: dict[str, Any]) -> None:
    if config_name == "deploy":
        validate_deploy(config_path, data)
    elif config_name == "monitor":
        validate_monitor(config_path, data)
    elif config_name == "flywheel":
        validate_flywheel(config_path, data)


def parse_duration(field: str, value: Any) -> int:
    if not isinstance(value, str):
        raise ConfigError(
            f"invalid duration for '{field}': expected string like 30s, 5m, 1h"
        )
    match = DURATION_RE.match(value)
    if not match:
        raise ConfigError(
            f"invalid duration for '{field}': '{value}' "
            "(expected <number><unit>, units: s, m, h, d)"
        )
    amount = int(match.group(1))
    unit = match.group(2)
    return amount * DURATION_SECONDS[unit]


def normalize_durations(config_name: str, data: dict[str, Any]) -> dict[str, Any]:
    normalized = dict(data)
    for field in DURATION_FIELDS.get(config_name, ()):
        if field in normalized:
            normalized[f"{field}_seconds"] = parse_duration(field, normalized[field])
    return normalized


def load_config(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    repo_root = resolve_repo_root(args.repo)
    config_file = resolve_config_path(args.name, repo_root, args.config)

    if not config_file.exists():
        if args.optional:
            return {}, 0
        raise ConfigError(
            f"missing config file: {config_file} "
            f"(create {repo_root / '.spellbook' / (args.name + '.yaml')})"
        )

    raw_config = load_simple_yaml(config_file)
    validate_config(args.name, config_file, raw_config)
    return normalize_durations(args.name, raw_config), 0


def main() -> int:
    args = parse_args()
    try:
        payload, code = load_config(args)
    except ConfigError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        if str(exc).startswith("missing config file:"):
            return 2
        return 1

    print(json.dumps(payload, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
