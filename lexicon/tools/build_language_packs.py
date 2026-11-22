#!/usr/bin/env python3
"""Build the Rust language packs from the single lexicon language sources.

Sources (one directory per language, never copied into Rust code):
  lexicon/languages/<lang>/stopwords.txt
  lexicon/languages/<lang>/adpositions.txt
  lexicon/languages/<lang>/articles.txt
  lexicon/languages/<lang>/pronouns.txt
  lexicon/languages/<lang>/possessives.txt
  lexicon/languages/<lang>/pronoun_clitics.txt
  lexicon/languages/<lang>/pro_drop_aux.txt
  lexicon/languages/<lang>/subordinate_markers.txt
  lexicon/languages/<lang>/negative_particles.txt
  lexicon/languages/<lang>/contrast_openers.txt
  lexicon/languages/<lang>/entity_blocklist.txt
  lexicon/languages/<lang>/place_candidate_blocklist.txt
  lexicon/languages/<lang>/place_candidate_token_blocklist.txt
  lexicon/languages/<lang>/admin_regions.txt
  lexicon/languages/<lang>/country_names.txt
  lexicon/languages/<lang>/generic_place_labels.txt
  lexicon/languages/<lang>/function_words.txt

`closed_class` is the union of the closed-class files and `blocked` is the
curated class-noun/verb blocklist: together they are the set the Rust extractor
uses to break a candidate phrase. `function_words` keeps the wider Go
sentence-start list in the pack for reference and for the Go pipeline; it is
NOT a Rust exclusion list, because it also contains open-class words ("great",
"mountains") that belong inside proper names.

Outputs:
  lexicon/packs/<lang>.json          one deterministic pack per language
  lexicon/packs/index.json           the supported language list
  semantic-core/src/generated_language_packs.rs
                                     include_str! table consumed by Rust

Run from the BrainRust root:
  python3 lexicon/tools/build_language_packs.py
  python3 lexicon/tools/build_language_packs.py --check

CI runs `--check` and fails when the generated files differ from the committed
ones, so a data edit can never leave a stale pack behind.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LANGUAGES = ROOT / "lexicon" / "languages"
PACKS = ROOT / "lexicon" / "packs"
PROFILES = ROOT / "lexicon" / "profiles"
GENERATED = ROOT / "semantic-core" / "src" / "generated_language_packs.rs"
GENERATED_PROFILES = ROOT / "semantic-core" / "src" / "generated_language_profiles.rs"
GO_PROFILES = (
    ROOT.parent
    / "refactored"
    / "internal"
    / "capabilities"
    / "linguistics"
    / "profiles.generated.json"
)
LEGAL = ROOT / "lexicon" / "legal"
NUMBER_WORDS = ROOT / "lexicon" / "number_words.json"
RELATIVE_TIME = ROOT / "lexicon" / "relative_time.json"
GIVEN_NAMES = ROOT / "lexicon" / "given_names.json"
PRONOUN_FIELDS = {"surface", "person", "gender", "number", "animacy", "case", "kind"}
REFERENCE_TERM_LANGUAGES = ("de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr")
REFERENCE_TERM_SCHEMA = "discourse_reference_terms.v1"
REFERENCE_TERM_FILE = ROOT / "lexicon" / "discourse_reference_terms_v1.json"
CHART_CUES = ROOT / "lexicon" / "chart_cues.json"
CHART_CUE_LANGUAGES = ("de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr")
ENTITY_TYPE_TERMS = ROOT / "lexicon" / "entity_type_terms.json"
ADDITIVE_ENTITY_TYPES = (
    "ANIMAL", "FOOD", "VEHICLE", "LAW", "COURT_CASE",
    "NORP", "DISEASE", "TECHNOLOGY", "ARTWORK", "CELESTIAL",
    "MEDIA_OUTLET",
)
LEGAL_INDEX = LEGAL / "index.json"
GENERATED_LEGAL = ROOT / "semantic-core" / "src" / "generated_legal_packs.rs"

# Closed-class inventories that already live in every language directory.
# Adding a word to any of these is a data edit; the pack follows automatically.
CLOSED_CLASS_FILES = (
    "stopwords.txt",
    "adpositions.txt",
    "articles.txt",
    "pronouns.txt",
    "possessives.txt",
    "pronoun_clitics.txt",
    "pro_drop_aux.txt",
    "subordinate_markers.txt",
    "negative_particles.txt",
    "contrast_openers.txt",
)

# Named sets the Go consumers ask for by name. The first nine mirror
# `extraWordSetFiles`; `adpositions` and `negative_phrases` are Rust-owned
# additions and are exposed for completeness.
WORD_SET_FILES = (
    "subordinate_markers",
    "pronouns",
    "articles",
    "possessives",
    "pronoun_clitics",
    "pro_drop_aux",
    "contrast_openers",
    "numeric_overlay_words",
    "numeric_overlay_context",
    "adpositions",
    "negative_phrases",
    "admin_regions",
    "country_names",
    "generic_place_labels",
)

DEFAULT_PHRASE_POLICY = {
    "min_words": 2,
    "max_words": 16,
    "max_results": 5,
    "reject_verbs_when_all": True,
}


def read_words(path: Path) -> list[str]:
    if not path.is_file():
        return []
    words: set[str] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip().lower()
        if line:
            words.add(line)
    return sorted(words)


def build_pack(language: str) -> dict:
    directory = LANGUAGES / language
    closed_class: set[str] = set()
    for name in CLOSED_CLASS_FILES:
        closed_class.update(read_words(directory / name))
    return {
        "schema": "language_pack.v1",
        "language": language,
        "source": f"lexicon/languages/{language}",
        "closed_class": sorted(closed_class),
        # Words that name a class, not an entity ("company", "contains"):
        # curated per language, never borrowed from another language.
        "blocked": read_words(directory / "entity_blocklist.txt"),
        # Wider Go sentence-start list, kept as data only. It contains
        # open-class words ("great", "mountains") and is NOT a filter set.
        "function_words": read_words(directory / "function_words.txt"),
        "stopwords": read_words(directory / "stopwords.txt"),
    }


def fnv1a64(data: bytes) -> int:
    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def read_phrase_policy(path: Path) -> dict:
    policy = dict(DEFAULT_PHRASE_POLICY)
    if not path.is_file():
        return policy
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line or "=" not in line:
            continue
        key, value = (part.strip() for part in line.split("=", 1))
        if key in ("min_words", "max_words", "max_results"):
            try:
                policy[key] = int(value)
            except ValueError:
                continue
        elif key == "reject_verbs_when_all":
            policy[key] = value.lower() in ("1", "true", "yes", "on")
    return policy


def build_profile(language: str) -> dict:
    directory = LANGUAGES / language
    word_sets = {
        name: read_words(directory / f"{name}.txt")
        for name in WORD_SET_FILES
        if (directory / f"{name}.txt").is_file()
    }
    return {
        "schema": "language_profile.v1",
        "language": language,
        "source": f"lexicon/languages/{language}",
        "stopwords": read_words(directory / "stopwords.txt"),
        "function_words": read_words(directory / "function_words.txt"),
        "entity_blocklist": read_words(directory / "entity_blocklist.txt"),
        # Map/geocode gate, after NER: whole queries that are never a place.
        "place_candidate_blocklist": read_words(
            directory / "place_candidate_blocklist.txt"
        ),
        # Tokens that disqualify a whole candidate wherever they appear.
        "place_candidate_token_blocklist": read_words(
            directory / "place_candidate_token_blocklist.txt"
        ),
        "negative_particles": read_words(directory / "negative_particles.txt"),
        "visual_verbs": read_words(directory / "visual_verbs.txt"),
        "verb_suffixes": read_words(directory / "verb_morphology.txt"),
        "word_sets": word_sets,
        "phrase_policy": read_phrase_policy(directory / "phrase_policy.txt"),
    }


def validate_number_words(languages: list[str]) -> None:
    """Every pack language needs grammar data for the ICU word scanner."""
    data = json.loads(NUMBER_WORDS.read_text(encoding="utf-8"))
    if data.get("schema") != "number_words.v1":
        raise SystemExit(f"{NUMBER_WORDS}: expected schema number_words.v1")
    rules = data.get("languages", {})
    missing = [language for language in languages if language != "fallback" and language not in rules]
    if missing:
        raise SystemExit(
            "lexicon/number_words.json is missing languages: " + ", ".join(missing)
        )
    for language in languages:
        if language == "fallback":
            continue
        entry = rules[language]
        for key in ("conjunctions", "approximators", "fraction_articles", "fractions", "negatives"):
            if key not in entry:
                raise SystemExit(
                    f"lexicon/number_words.json: {language} has no {key!r}"
                )


def validate_relative_time(languages: list[str]) -> None:
    """Every pack language needs units, direction cues and named relatives."""
    data = json.loads(RELATIVE_TIME.read_text(encoding="utf-8"))
    if data.get("schema") != "relative_time.v1":
        raise SystemExit(f"{RELATIVE_TIME}: expected schema relative_time.v1")
    rules = data.get("languages", {})
    missing = [language for language in languages if language != "fallback" and language not in rules]
    if missing:
        raise SystemExit(
            "lexicon/relative_time.json is missing languages: " + ", ".join(missing)
        )
    for language in languages:
        if language == "fallback":
            continue
        entry = rules[language]
        for key in ("units", "after", "before", "named", "span"):
            if not entry.get(key):
                raise SystemExit(
                    f"lexicon/relative_time.json: {language} has no {key!r}"
                )


def validate_structured_pronouns(languages: list[str]) -> None:
    """Keep typed pronoun inventories in sync with the closed-class sources."""
    for language in languages:
        if language == "fallback":
            continue
        source = LANGUAGES / language / "pronouns.txt"
        structured = LANGUAGES / language / "pronouns.json"
        if not structured.is_file():
            raise SystemExit(f"{structured}: missing structured pronoun inventory")
        payload = json.loads(structured.read_text(encoding="utf-8"))
        if payload.get("schema") != "pronouns.v1" or payload.get("language") != language:
            raise SystemExit(f"{structured}: expected pronouns.v1 for {language}")
        rows = payload.get("pronouns")
        if not isinstance(rows, list):
            raise SystemExit(f"{structured}: pronouns must be an array")
        surfaces = []
        for row in rows:
            if set(row) != PRONOUN_FIELDS:
                raise SystemExit(f"{structured}: each row must contain {sorted(PRONOUN_FIELDS)}")
            if not isinstance(row["surface"], str) or not row["surface"].strip():
                raise SystemExit(f"{structured}: surface must be non-empty")
            for field, allowed in {
                "person": {"1", "2", "3", "unknown"},
                "gender": {"masculine", "feminine", "neuter", "common", "unknown"},
                "number": {"singular", "plural", "unknown"},
                "animacy": {"animate", "inanimate", "unknown"},
                "case": {"nominative", "accusative", "dative", "genitive", "oblique", "unknown"},
                "kind": {"personal", "possessive", "demonstrative", "relative", "reflexive"},
            }.items():
                if row[field] not in allowed:
                    raise SystemExit(f"{structured}: invalid {field} on {row['surface']!r}")
            surfaces.append(row["surface"].lower())
        expected = read_words(source)
        # German, Dutch and Turkish had no dedicated pronouns.txt; their
        # curated pronouns.json inventory is deliberately additive metadata so
        # the generated language-pack bytes remain unchanged.
        if source.is_file() and sorted(set(surfaces)) != expected:
            raise SystemExit(f"{structured}: surfaces must exactly match {source}")
        if not surfaces:
            raise SystemExit(f"{structured}: pronoun inventory must not be empty")


def validate_given_names(languages: list[str]) -> None:
    """Validate the curated names shared by NER and discourse resolution."""
    data = json.loads(GIVEN_NAMES.read_text(encoding="utf-8"))
    if data.get("schema") != "given_names.v1":
        raise SystemExit(f"{GIVEN_NAMES}: expected schema given_names.v1")
    entries = data.get("languages", {})
    missing = [language for language in languages if language != "fallback" and language not in entries]
    if missing:
        raise SystemExit(f"{GIVEN_NAMES}: missing languages: " + ", ".join(missing))
    for language in languages:
        if language == "fallback":
            continue
        entry = entries[language]
        if set(entry) != {"feminine", "masculine", "unknown"}:
            raise SystemExit(f"{GIVEN_NAMES}: {language} must define feminine, masculine, and unknown lists")
        normalized: dict[str, set[str]] = {}
        for gender in ("feminine", "masculine", "unknown"):
            names = entry[gender]
            if not isinstance(names, list) or any(not isinstance(name, str) or not name.strip() for name in names):
                raise SystemExit(f"{GIVEN_NAMES}: {language}.{gender} must contain non-empty strings")
            folded = [name.casefold() for name in names]
            if len(folded) != len(set(folded)):
                raise SystemExit(f"{GIVEN_NAMES}: duplicate {gender} name in {language}")
            normalized[gender] = set(folded)
        overlap = (normalized["feminine"] & normalized["masculine"]) | (
            normalized["unknown"] & (normalized["feminine"] | normalized["masculine"])
        )
        if overlap:
            raise SystemExit(f"{GIVEN_NAMES}: ambiguous gender entries in {language}: {sorted(overlap)}")


def build() -> tuple[list[str], list[str], dict[Path, str]]:
    languages = sorted(
        path.name
        for path in LANGUAGES.iterdir()
        if path.is_dir() and path.name != "fallback"
    )
    files: dict[Path, str] = {}
    for language in languages:
        files[PACKS / f"{language}.json"] = (
            json.dumps(build_pack(language), ensure_ascii=False, indent=2) + "\n"
        )
    files[PACKS / "index.json"] = (
        json.dumps(
            {"schema": "language_packs.v1", "languages": languages},
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )

    # Go needs every language the repository enumerates, including the
    # cross-linguistic fallback. The profile keeps the named sets distinct;
    # the fused pack above is only the Rust extractor's exclusion index.
    profile_languages = sorted(
        path.name for path in LANGUAGES.iterdir() if path.is_dir()
    )
    validate_number_words(profile_languages)
    validate_relative_time(profile_languages)
    validate_structured_pronouns(profile_languages)
    validate_given_names(profile_languages)
    profiles = []
    for language in profile_languages:
        profile = build_profile(language)
        profiles.append(profile)
        files[PROFILES / f"{language}.json"] = (
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n"
        )
    files[PROFILES / "index.json"] = (
        json.dumps(
            {"schema": "language_profiles.v1", "languages": profile_languages},
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    profile_entries = "\n".join(
        f'    ("{language}", include_str!("../../lexicon/profiles/{language}.json")),'
        for language in profile_languages
    )
    files[GENERATED_PROFILES] = (
        "// @generated by lexicon/tools/build_language_packs.py; do not edit.\n"
        "// One named-set profile per lexicon/languages/<lang> directory.\n\n"
        "#[rustfmt::skip]\n"
        "pub const PROFILES: &[(&str, &str)] = &[\n"
        f"{profile_entries}\n"
        "];\n"
    )
    version_hash = 0xCBF29CE484222325
    for language in profile_languages:
        payload = files[PROFILES / f"{language}.json"].encode("utf-8")
        for byte in language.encode("utf-8") + payload:
            version_hash ^= byte
            version_hash = (version_hash * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    go_payload = {
        "schema": "go_lexicon_profiles.v1",
        "source": "BrainRust/lexicon/profiles",
        "rust_schema_version": f"language_profiles.v1-{version_hash:016x}",
        "profiles": profiles,
    }
    files[GO_PROFILES] = json.dumps(go_payload, ensure_ascii=False, indent=2) + "\n"
    entries = "\n".join(
        f'    ("{language}", include_str!("../../lexicon/packs/{language}.json")),'
        for language in languages
    )
    files[GENERATED] = (
        "// @generated by lexicon/tools/build_language_packs.py; do not edit.\n"
        "// One pack per lexicon/languages/<lang> directory. Adding a language is\n"
        "// a data directory plus a regenerated table, never a Rust match arm.\n\n"
        "pub const PACKS: &[(&str, &str)] = &[\n"
        f"{entries}\n"
        "];\n"
    )
    legal_languages = sorted(path.stem for path in LEGAL.glob("*.json") if path.name != "index.json")
    files[LEGAL_INDEX] = (
        json.dumps(
            {"schema": "legal_packs.v1", "languages": legal_languages},
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    legal_entries = "\n".join(
        f'    ("{language}", include_str!("../../lexicon/legal/{language}.json")),'
        for language in legal_languages
    )
    files[GENERATED_LEGAL] = (
        "// @generated by lexicon/tools/build_language_packs.py; do not edit.\n"
        "// Optional domain packs: a language without a legal pack produces no\n"
        "// legal facts and no micro summaries instead of importing another\n"
        "// language's vocabulary.\n\n"
        "pub const LEGAL_PACKS: &[(&str, &str)] = &[\n"
        f"{legal_entries}\n"
        "];\n"
    )
    return languages, profile_languages, files


def validate_auxiliary_lexicons() -> list[str]:
    """Keep additive semantic lexicons under the canonical language-data gate."""
    tools = ROOT / "lexicon" / "tools"
    validators = (
        ("check_roles.py", "role-check", lambda module: [
            error for language in ("en", "it", "es", "de", "fr", "id", "nl", "pl", "pt", "ru", "tr") for error in module.check(language)
        ]),
        ("check_event_lexicons.py", "event-check", lambda module: [
            error
            for language in ("en", "it", "es", "fr", "pt", "de", "nl", "id", "pl", "ru", "tr")
            for error in module.validate(
                ROOT / "lexicon" / "events" / f"{language}.json", language
            )
        ]),
    )
    errors = []
    for filename, module_name, validate in validators:
        spec = importlib.util.spec_from_file_location(module_name, tools / filename)
        if spec is None or spec.loader is None:
            errors.append(f"cannot load lexicon validator: {filename}")
            continue
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        errors.extend(validate(module))
    errors.extend(validate_discourse_reference_terms())
    errors.extend(validate_chart_cues())
    errors.extend(validate_entity_type_terms())
    return errors


def validate_entity_type_terms() -> list[str]:
    """Validate the additive entity-type vocabulary for all 11 languages."""
    try:
        data = json.loads(ENTITY_TYPE_TERMS.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{ENTITY_TYPE_TERMS.relative_to(ROOT)}: {error}"]
    errors: list[str] = []
    if data.get("schema") != "entity_type_terms.v1":
        errors.append(f"{ENTITY_TYPE_TERMS.relative_to(ROOT)}: expected entity_type_terms.v1")
    if set(data.get("types", [])) != set(ADDITIVE_ENTITY_TYPES):
        errors.append(
            f"{ENTITY_TYPE_TERMS.relative_to(ROOT)}: types must be "
            + ", ".join(ADDITIVE_ENTITY_TYPES)
        )
    domains = data.get("media_outlet_domains")
    if not isinstance(domains, dict) or set(domains) != set(CHART_CUE_LANGUAGES):
        errors.append(
            f"{ENTITY_TYPE_TERMS.relative_to(ROOT)}: media_outlet_domains must be "
            + ", ".join(CHART_CUE_LANGUAGES)
        )
    else:
        for language, entries in domains.items():
            if not isinstance(entries, list) or not entries:
                errors.append(f"{language}.media_outlet_domains: expected a non-empty array")
                continue
            if any(
                not isinstance(domain, str)
                or "." not in domain
                or any(ch.isspace() for ch in domain)
                for domain in entries
            ):
                errors.append(f"{language}.media_outlet_domains: entries must be domains")
    languages = data.get("languages")
    if not isinstance(languages, dict) or set(languages) != set(CHART_CUE_LANGUAGES):
        errors.append(
            f"{ENTITY_TYPE_TERMS.relative_to(ROOT)}: languages must be "
            + ", ".join(CHART_CUE_LANGUAGES)
        )
        return errors
    for language, inventory in languages.items():
        if not isinstance(inventory, dict) or set(inventory) != set(ADDITIVE_ENTITY_TYPES):
            errors.append(f"{language}: expected every additive entity type")
            continue
        for kind, terms in inventory.items():
            if not isinstance(terms, list) or not terms:
                errors.append(f"{language}.{kind}: expected a non-empty array")
                continue
            if any(not isinstance(term, str) or not term.strip() for term in terms):
                errors.append(f"{language}.{kind}: terms must be non-empty strings")
    return errors


def validate_chart_cues() -> list[str]:
    """Validate the chart-specific word tables consumed by chart_rules."""
    try:
        data = json.loads(CHART_CUES.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{CHART_CUES.relative_to(ROOT)}: {error}"]

    errors: list[str] = []
    if data.get("schema") != "chart_cues.v1":
        errors.append(f"{CHART_CUES.relative_to(ROOT)}: expected chart_cues.v1")
    if sorted(data.get("languages", [])) != sorted(CHART_CUE_LANGUAGES):
        errors.append(
            f"{CHART_CUES.relative_to(ROOT)}: languages must be "
            + ", ".join(CHART_CUE_LANGUAGES)
        )

    global_cues = data.get("global")
    if not isinstance(global_cues, dict):
        return errors + [f"{CHART_CUES.relative_to(ROOT)}: global must be an object"]
    for key in (
        "hard_vetoes",
        "approximators",
        "approximator_pairs",
        "rate_cues",
        "rank_veto_cues",
        "subject_stopwords",
        "subject_verb_cutoffs",
        "count_unit_cues",
        "display_unit_cues",
        "monthly_pairs",
        "yearly_cues",
        "paired_series_rules",
    ):
        value = global_cues.get(key)
        if not isinstance(value, list) or not value:
            errors.append(f"global.{key}: expected a non-empty array")
    for key in ("currency_cues", "length_cues"):
        value = global_cues.get(key)
        if not isinstance(value, dict) or not value:
            errors.append(f"global.{key}: expected a non-empty object")
    paired_rules = global_cues.get("paired_series_rules")
    if isinstance(paired_rules, list):
        for index, rule in enumerate(paired_rules):
            if not isinstance(rule, dict):
                errors.append(f"global.paired_series_rules[{index}]: expected an object")
                continue
            groups = rule.get("any_in_each")
            if not isinstance(groups, list) or not groups or any(
                not isinstance(group, list) or not group for group in groups
            ):
                errors.append(
                    f"global.paired_series_rules[{index}].any_in_each: expected non-empty cue groups"
                )
            repeated = rule.get("repeated_units", [])
            if not isinstance(repeated, list):
                errors.append(f"global.paired_series_rules[{index}].repeated_units: expected an array")
    for key, rows in global_cues.items():
        if key in ("currency_cues", "length_cues", "paired_series_rules"):
            continue
        if not isinstance(rows, list):
            continue
        for index, row in enumerate(rows):
            values = row if isinstance(row, list) else [row]
            if not values or any(not isinstance(item, str) or not item.strip() for item in values):
                errors.append(f"global.{key}[{index}]: cues must be non-empty strings")

    by_language = data.get("by_language")
    if not isinstance(by_language, dict) or set(by_language) != set(CHART_CUE_LANGUAGES):
        errors.append(
            f"{CHART_CUES.relative_to(ROOT)}: by_language must exactly cover "
            + ", ".join(CHART_CUE_LANGUAGES)
        )
        return errors
    for language, cues in by_language.items():
        if not isinstance(cues, dict):
            errors.append(f"by_language.{language}: expected an object")
            continue
        for key in (
            "series_context_cues",
            "survey_cues",
            "series_metric_stems",
            "semicolon_metric_stems",
            "share_context_cues",
            "growth_cues",
            "scenario_cues",
        ):
            rows = cues.get(key)
            if not isinstance(rows, list) or not rows:
                errors.append(f"by_language.{language}.{key}: expected a non-empty array")
                continue
            if any(not isinstance(row, str) or not row.strip() for row in rows):
                errors.append(f"by_language.{language}.{key}: cues must be non-empty strings")
    return errors


def validate_discourse_reference_terms() -> list[str]:
    """Validate the language-scoped terms consumed by the discourse resolver."""
    try:
        data = json.loads(REFERENCE_TERM_FILE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{REFERENCE_TERM_FILE.relative_to(ROOT)}: {error}"]

    errors = []
    if data.get("schema") != REFERENCE_TERM_SCHEMA:
        errors.append(f"{REFERENCE_TERM_FILE.relative_to(ROOT)}: expected {REFERENCE_TERM_SCHEMA}")
    languages = data.get("languages")
    if not isinstance(languages, dict) or set(languages) != set(REFERENCE_TERM_LANGUAGES):
        errors.append(
            f"{REFERENCE_TERM_FILE.relative_to(ROOT)}: languages must be "
            f"{', '.join(REFERENCE_TERM_LANGUAGES)}"
        )
        return errors

    for language, inventory in languages.items():
        if not isinstance(inventory, dict):
            errors.append(f"{language}: expected an object")
            continue
        for key, allowed_fields in (
            ("terms", {"text", "type", "kind", "number"}),
            ("phrases", {"text", "kind", "number"}),
        ):
            rows = inventory.get(key)
            if not isinstance(rows, list):
                errors.append(f"{language}.{key}: expected an array")
                continue
            seen = set()
            for index, row in enumerate(rows):
                label = f"{language}.{key}[{index}]"
                if not isinstance(row, dict) or set(row) != allowed_fields:
                    errors.append(f"{label}: fields must be {', '.join(sorted(allowed_fields))}")
                    continue
                text = row["text"]
                if not isinstance(text, str) or not text or text != text.lower():
                    errors.append(f"{label}.text: expected a non-empty lowercase string")
                    continue
                if text in seen:
                    errors.append(f"{label}.text: duplicate {text!r}")
                seen.add(text)
                if not isinstance(row["kind"], str) or row["kind"] not in {"PERSON", "ORG", "ANY"}:
                    errors.append(f"{label}.kind: invalid value {row['kind']!r}")
                if not isinstance(row["number"], str) or row["number"] not in {"singular", "plural", "unknown"}:
                    errors.append(f"{label}.number: invalid value {row['number']!r}")
                if key == "terms" and (
                    not isinstance(row["type"], str)
                    or row["type"] not in {"pronoun", "nominal_reference"}
                ):
                    errors.append(f"{label}.type: invalid value {row['type']!r}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed packs match the sources; never writes",
    )
    args = parser.parse_args()

    auxiliary_errors = validate_auxiliary_lexicons()
    if auxiliary_errors:
        print("semantic lexicons are invalid:", file=sys.stderr)
        print("\n".join(f"  {error}" for error in auxiliary_errors), file=sys.stderr)
        return 1

    languages, profile_languages, files = build()
    stale = [
        path for path, content in files.items()
        if not path.is_file() or path.read_text(encoding="utf-8") != content
    ]
    def display(path: Path) -> str:
        try:
            return str(path.relative_to(ROOT.parent))
        except ValueError:
            return str(path)

    if args.check:
        if stale:
            print("language data is stale; run the generator:", file=sys.stderr)
            for path in stale:
                print(f"  {display(path)}", file=sys.stderr)
            return 1
        print(
            f"language packs: {len(languages)} up to date -> {', '.join(languages)}; "
            f"profiles: {len(profile_languages)}; Go snapshot up to date; "
            "role lexicons valid (11 languages); event lexicons valid (11 languages); "
            "discourse reference lexicon valid (11 languages)"
        )
        return 0

    PACKS.mkdir(parents=True, exist_ok=True)
    for path, content in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    legal = sorted(path.stem for path in LEGAL.glob("*.json") if path.name != "index.json")
    print(f"language packs: {len(languages)} -> {', '.join(languages)}")
    print(f"language profiles: {len(profile_languages)} -> {', '.join(profile_languages)}")
    print(f"Go lexicon snapshot: {display(GO_PROFILES)}")
    print(f"legal packs: {len(legal)} -> {', '.join(legal)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
