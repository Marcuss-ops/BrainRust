#!/usr/bin/env python3
"""Validate the initial, language-scoped event trigger lexicons."""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REQUIRED_TYPES = {
    "birth", "death", "arrest", "trial_verdict", "election",
    "appointment_resignation", "founding", "acquisition_merger",
    "launch_release", "accident_disaster", "attack_battle",
    "match_competition_result",
}
OPTIONAL_TYPES = {"agreement"}
ALLOWED_VOICES = {"active", "passive", "nominal"}
ALLOWED_ROLES = {"agent", "patient", "winner", "signatory", "counterparty", "vehicle"}


def validate(path: Path, language: str) -> list[str]:
    errors = []
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != "event_triggers.v1" or data.get("language") != language:
        errors.append(f"{path}: invalid schema or language")
    events = data.get("event_types", {})
    if not REQUIRED_TYPES <= set(events) or not set(events) <= REQUIRED_TYPES | OPTIONAL_TYPES:
        errors.append(f"{path}: expected the 12 core event types and optional agreement")
    for event_type, triggers in events.items():
        if not isinstance(triggers, list) or not triggers:
            errors.append(f"{path}: {event_type} has no triggers")
            continue
        for trigger in triggers:
            if not all(isinstance(trigger.get(key), str) and trigger[key] for key in ("surface", "lemma")):
                errors.append(f"{path}: {event_type} trigger needs surface and lemma")
            if trigger.get("voice") not in ALLOWED_VOICES:
                errors.append(f"{path}: {event_type} has invalid voice")
            roles = trigger.get("roles", {})
            if not isinstance(roles, dict) or not set(roles) <= ALLOWED_ROLES:
                errors.append(f"{path}: {event_type} has invalid participant roles")
            if any(value not in {"subject", "object"} for value in roles.values()):
                errors.append(f"{path}: {event_type} has invalid argument mapping")
    return errors


def main() -> int:
    errors = []
    languages = ("en", "it", "es", "fr", "pt", "de", "nl", "id", "pl", "ru", "tr")
    for language in languages:
        errors.extend(validate(ROOT / "lexicon" / "events" / f"{language}.json", language))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"event trigger lexicons: {len(languages)} languages; 12 core event types each; valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
