#!/usr/bin/env python3
"""Validate the shared mediaexec.v1 goldens against the versioned schemas.

Run from anywhere:
    python3 contract/mediaexec.v1/validate.py

Requires jsonschema (see requirements.txt). The validator is deliberately
strict about the golden set: a missing or extra fixture is a contract change,
not a silent reduction in coverage.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from jsonschema import Draft7Validator
except ImportError:  # pragma: no cover - exercised by the CI setup path
    sys.exit(
        "jsonschema is required: install it from "
        "contract/mediaexec.v1/requirements.txt"
    )

ROOT = Path(__file__).resolve().parent
EXPECTED = {"probe", "cut_batch", "normalize"}


def load(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def fail(message: str) -> None:
    print(f"mediaexec.v1 contract: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate(validator: Draft7Validator, document: dict, name: str, kind: str) -> None:
    errors = sorted(validator.iter_errors(document), key=lambda error: list(error.path))
    if not errors:
        return
    details = []
    for error in errors:
        location = "/".join(str(part) for part in error.path) or "<root>"
        details.append(f"{location}: {error.message}")
    fail(f"{name} {kind} does not match the schema:\n  " + "\n  ".join(details))


def main() -> None:
    request_schema = load(ROOT / "request.schema.json")
    response_schema = load(ROOT / "response.schema.json")
    Draft7Validator.check_schema(request_schema)
    Draft7Validator.check_schema(response_schema)
    request_validator = Draft7Validator(request_schema)
    response_validator = Draft7Validator(response_schema)

    goldens = sorted((ROOT / "goldens").glob("*.json"))
    found = {path.stem for path in goldens}
    if found != EXPECTED:
        fail(
            f"golden set drift: found {sorted(found)}, expected {sorted(EXPECTED)}"
        )

    for path in goldens:
        document = load(path)
        if "request" not in document or "response" not in document:
            fail(f"{path.name}: expected request and response envelopes")
        request = document["request"]
        response = document["response"]
        validate(request_validator, request, path.stem, "request")
        validate(response_validator, response, path.stem, "response")

        if request.get("version") != "mediaexec.v1":
            fail(f"{path.name}: request.version is not mediaexec.v1")
        if request.get("operation") != path.stem:
            fail(f"{path.name}: request.operation does not match the golden name")
        if response.get("operation") != path.stem:
            fail(f"{path.name}: response.operation does not match the golden name")
        if response.get("ok") is not True:
            fail(f"{path.name}: the shared golden set only carries success responses")

    print(f"✅ mediaexec.v1 contract: {len(goldens)} shared goldens validated")


if __name__ == "__main__":
    main()
