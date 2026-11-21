#!/usr/bin/env python3
"""Validate the curated person-role vocabulary seeds."""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = {
    "chief_executive", "president", "vice_president", "chief_operating_officer",
    "chief_financial_officer", "chief_technology_officer", "founder", "co_founder",
    "director", "board_member", "chairperson", "secretary", "treasurer", "minister",
    "prime_minister", "mayor", "governor", "senator", "representative", "judge",
    "prosecutor", "lawyer", "police_chief", "coach", "captain", "player", "manager",
    "spokesperson", "journalist", "editor", "professor", "researcher", "scientist",
    "doctor", "engineer", "artist", "actor", "singer", "author", "owner",
}
SCOPES = {"org", "place", "none"}
GENDERS = {"masculine", "feminine", "common", "unknown"}
NUMBERS = {"singular", "plural", "unknown"}


def check(language: str) -> list[str]:
    path = ROOT / "lexicon" / "roles" / f"{language}.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    errors = []
    if data.get("schema") != "role_lexicon.v1" or data.get("language") != language:
        errors.append(f"{path}: wrong schema/language")
    roles = data.get("roles", [])
    codes = [row.get("role_code") for row in roles]
    if set(codes) != EXPECTED or len(codes) != len(EXPECTED):
        errors.append(f"{path}: role codes differ from the 40-code seed")
    for role in roles:
        if role.get("scope") not in SCOPES or not role.get("surfaces"):
            errors.append(f"{path}: invalid scope or missing surfaces for {role.get('role_code')}")
        seen_surfaces = set()
        for surface in role.get("surfaces", []):
            if not surface.get("text") or surface.get("gender") not in GENDERS or surface.get("number") not in NUMBERS or not surface.get("case"):
                errors.append(f"{path}: invalid surface for {role.get('role_code')}")
            key = surface.get("text", "").casefold()
            if key in seen_surfaces:
                errors.append(f"{path}: duplicate surface {surface.get('text')!r} for {role.get('role_code')}")
            seen_surfaces.add(key)
    markers = data.get("tense_markers", {})
    if not all(markers.get(key) for key in ("former", "acting", "appointed", "resigned")):
        errors.append(f"{path}: missing tense marker class")
    return errors


def main() -> int:
    languages = ("en", "it", "es", "de", "fr", "id", "nl", "pl", "pt", "ru", "tr")
    errors = [error for lang in languages for error in check(lang)]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"role lexicons: 40 codes each in {', '.join(languages)}; valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
