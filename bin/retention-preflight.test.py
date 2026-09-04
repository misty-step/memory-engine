#!/usr/bin/env python3
"""Executable, dependency-free tests for retention-preflight.py."""

from __future__ import annotations

import importlib.util
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("retention-preflight.py")
SPEC = importlib.util.spec_from_file_location("retention_preflight", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {MODULE_PATH}")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class LifecycleClientError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.response = {"Error": {"Code": code}}


class StubClient:
    def __init__(self, versioning: dict, lifecycle: dict | Exception) -> None:
        self.versioning = versioning
        self.lifecycle = lifecycle
        self.calls: list[str] = []

    def get_bucket_versioning(self, *, Bucket: str) -> dict:
        self.calls.append("versioning")
        return self.versioning

    def get_bucket_lifecycle_configuration(self, *, Bucket: str) -> dict:
        self.calls.append("lifecycle")
        if isinstance(self.lifecycle, Exception):
            raise self.lifecycle
        return self.lifecycle


def assert_refused(name: str, client: StubClient) -> None:
    try:
        MODULE.check(client, "scry-backups")
    except SystemExit as error:
        assert "REFUSED" in str(error), f"{name}: refusal omitted REFUSED: {error}"
    else:
        raise AssertionError(f"{name}: expected SystemExit")


def assert_passes(name: str, client: StubClient) -> None:
    MODULE.check(client, "scry-backups")


def enabled_rule(**fields: object) -> dict:
    return {"ID": "retention", "Status": "Enabled", **fields}


def main() -> None:
    cases = [
        (
            "versioning-not-enabled",
            lambda: assert_refused(
                "versioning-not-enabled",
                StubClient(
                    {"Status": "Suspended"},
                    {"Rules": [enabled_rule(Expiration={"Days": 30})]},
                ),
            ),
        ),
        (
            "missing-lifecycle-configuration",
            lambda: assert_refused(
                "missing-lifecycle-configuration",
                StubClient(
                    {"Status": "Enabled"},
                    LifecycleClientError("NoSuchLifecycleConfiguration"),
                ),
            ),
        ),
        (
            "enabled-rule-without-expiration",
            lambda: assert_refused(
                "enabled-rule-without-expiration",
                StubClient({"Status": "Enabled"}, {"Rules": [enabled_rule()]}),
            ),
        ),
        (
            "non-scry-prefix",
            lambda: assert_refused(
                "non-scry-prefix",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Filter={"Prefix": "other/"},
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "narrow-scry-prefix",
            lambda: assert_refused(
                "narrow-scry-prefix",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Filter={"Prefix": "scry/archive/"},
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "current-expiration-only",
            lambda: assert_refused(
                "current-expiration-only",
                StubClient(
                    {"Status": "Enabled"},
                    {"Rules": [enabled_rule(Expiration={"Days": 30})]},
                ),
            ),
        ),
        (
            "noncurrent-expiration-only",
            lambda: assert_refused(
                "noncurrent-expiration-only",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Filter={"Prefix": "scry/"},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "current-expiration-without-days",
            lambda: assert_refused(
                "current-expiration-without-days",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"ExpiredObjectDeleteMarker": True},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "current-expiration-exceeds-thirty-days",
            lambda: assert_refused(
                "current-expiration-exceeds-thirty-days",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"Days": 31},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "noncurrent-expiration-without-days",
            lambda: assert_refused(
                "noncurrent-expiration-without-days",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={
                                    "NewerNoncurrentVersions": 1
                                },
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "noncurrent-expiration-exceeds-thirty-days",
            lambda: assert_refused(
                "noncurrent-expiration-exceeds-thirty-days",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={"NoncurrentDays": 31},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "noncurrent-expiration-with-version-threshold",
            lambda: assert_refused(
                "noncurrent-expiration-with-version-threshold",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={
                                    "NoncurrentDays": 30,
                                    "NewerNoncurrentVersions": 1,
                                },
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "bucket-wide-current-and-noncurrent",
            lambda: assert_passes(
                "bucket-wide-current-and-noncurrent",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
        (
            "scry-prefix-current-and-noncurrent",
            lambda: assert_passes(
                "scry-prefix-current-and-noncurrent",
                StubClient(
                    {"Status": "Enabled"},
                    {
                        "Rules": [
                            enabled_rule(
                                Filter={"Prefix": "scry/"},
                                Expiration={"Days": 30},
                                NoncurrentVersionExpiration={"NoncurrentDays": 30},
                            )
                        ]
                    },
                ),
            ),
        ),
    ]
    for name, case in cases:
        case()
        print(f"ok {name}")
    print("OK (all cases)")


if __name__ == "__main__":
    main()
