#!/usr/bin/env python3
"""Refuse Scry backups unless the Spaces bucket has safe retention policy."""

from __future__ import annotations

import os
from typing import Any


def _refuse(message: str) -> None:
    raise SystemExit(f"REFUSED: {message}")


def _error_code(error: BaseException) -> str | None:
    response = getattr(error, "response", None)
    if not isinstance(response, dict):
        return None
    details = response.get("Error")
    if not isinstance(details, dict):
        return None
    code = details.get("Code")
    return code if isinstance(code, str) else None


def _rule_prefix(rule: dict[str, Any]) -> str | None:
    """Return the rule's object scope, or None when it cannot be proven safe."""

    if "Filter" in rule:
        filter_value = rule["Filter"]
        if filter_value is None or filter_value == {}:
            return ""
        if not isinstance(filter_value, dict):
            return None
        prefix = filter_value.get("Prefix")
        if not isinstance(prefix, str):
            # Tag and And filters cannot prove that Scry objects are covered.
            return None
        return prefix

    # Prefix is the legacy S3 lifecycle shape. An omitted filter/prefix is
    # bucket-wide and therefore covers the scry/ upload prefix.
    prefix = rule.get("Prefix", "")
    return prefix if isinstance(prefix, str) else None


def _covers_uploads(prefix: str) -> bool:
    # Uploads are scry/<basename>. Only a bucket-wide rule or an exact scry/
    # prefix covers every object the uploader writes.
    return prefix in ("", "scry/")


def _has_30_day_action(rule: dict[str, Any], action: str, days_field: str) -> bool:
    value = rule.get(action)
    return (
        isinstance(value, dict)
        and value.get(days_field) == 30
        and (
            action != "NoncurrentVersionExpiration"
            or "NewerNoncurrentVersions" not in value
        )
    )


def check(client: Any, bucket: str) -> None:
    """Validate versioning and lifecycle retention for ``bucket``.

    A failed preflight raises ``SystemExit`` with a ``REFUSED:`` message so
    callers can fail closed before creating a database dump or uploading it.
    Other provider errors are allowed to propagate as operational failures.
    """

    versioning = client.get_bucket_versioning(Bucket=bucket)
    if versioning.get("Status") != "Enabled":
        _refuse(f"bucket {bucket!r} versioning is not Enabled")

    try:
        lifecycle = client.get_bucket_lifecycle_configuration(Bucket=bucket)
    except Exception as error:
        if _error_code(error) == "NoSuchLifecycleConfiguration":
            _refuse(f"bucket {bucket!r} has no lifecycle configuration")
        raise

    rules = lifecycle.get("Rules")
    if not isinstance(rules, list):
        _refuse(f"bucket {bucket!r} has no lifecycle rules")

    enabled_rules = [
        rule
        for rule in rules
        if isinstance(rule, dict) and rule.get("Status") == "Enabled"
    ]
    if not enabled_rules:
        _refuse(f"bucket {bucket!r} has no Enabled lifecycle rule")

    covering: list[dict[str, Any]] = []
    for rule in enabled_rules:
        prefix = _rule_prefix(rule)
        if prefix is None:
            _refuse("Enabled lifecycle rule has an unscoped filter")
        if _covers_uploads(prefix):
            covering.append(rule)

    if not covering:
        _refuse(f"bucket {bucket!r} has no Enabled lifecycle rule covering scry/ uploads")

    if not any(_has_30_day_action(rule, "Expiration", "Days") for rule in covering):
        _refuse("covering lifecycle rule has no 30-day current-object Expiration")
    if not any(
        _has_30_day_action(
            rule, "NoncurrentVersionExpiration", "NoncurrentDays"
        )
        for rule in covering
    ):
        _refuse("covering lifecycle rule has no 30-day NoncurrentVersionExpiration")


def main() -> None:
    import boto3
    from botocore.client import Config

    endpoint = os.environ["ENDPOINT"]
    bucket = os.environ["BUCKET"]
    client = boto3.client(
        "s3",
        endpoint_url=endpoint,
        region_name=os.environ.get("SCRY_BACKUP_REGION", "nyc3"),
        aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        config=Config(signature_version="s3v4"),
    )
    check(client, bucket)


if __name__ == "__main__":
    main()
