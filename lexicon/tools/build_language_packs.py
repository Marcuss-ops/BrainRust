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
        "negative_particles": read_words(directory / "negative_particles.txt"),
        "visual_verbs": read_words(directory / "visual_verbs.txt"),
        "verb_suffixes": read_words(directory / "verb_morphology.txt"),
        "word_sets": word_sets,
        "phrase_policy": read_phrase_policy(directory / "phrase_policy.txt"),
    }


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed packs match the sources; never writes",
    )
    args = parser.parse_args()

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
            f"profiles: {len(profile_languages)}; Go snapshot up to date"
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
