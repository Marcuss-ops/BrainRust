#!/usr/bin/env python3
"""One-shot migration helper: extract lexical tables from Rust sources into
the JSON lexicon files, byte-identical and order-preserving.

The Rust sources are the historical source of truth; this script copies every
string literal of the curated tables into `lexicon/*.json` so the knowledge
moves out of code without transcription drift. Re-running is idempotent for
the tables it owns.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

RUST_ROOT = Path(__file__).resolve().parents[2]
LEXICON_DIR = RUST_ROOT / "lexicon"


def strip_comments(text: str) -> str:
    return re.sub(r"//[^\n]*", "", text)


def parse_literals(blob: str):
    """Parse a Rust array literal body into Python str / list[str] values,
    preserving order. Nested `&[...]` arrays become nested lists."""
    items: list = []
    i = 0
    while i < len(blob):
        ch = blob[i]
        if ch == '"':
            j = i + 1
            out = []
            while j < len(blob):
                if blob[j] == "\\":
                    out.append(blob[j : j + 2])
                    j += 2
                    continue
                if blob[j] == '"':
                    break
                out.append(blob[j])
                j += 1
            raw = "".join(out)
            items.append(raw.replace('\\"', '"').replace("\\n", "\n"))
            i = j + 1
            continue
        if blob.startswith("&[", i) or ch == "[":
            open_at = i + (2 if blob.startswith("&[", i) else 1)
            depth = 1
            j = open_at
            while j < len(blob) and depth:
                if blob[j] == '"':
                    j += 1
                    while j < len(blob) and blob[j] != '"':
                        j += 2 if blob[j] == "\\" else 1
                elif blob[j] == "[":
                    depth += 1
                elif blob[j] == "]":
                    depth -= 1
                j += 1
            items.append(parse_literals(blob[open_at : j - 1]))
            i = j
            continue
        i += 1
    return items


def const_array(source: str, name: str):
    source = strip_comments(source)
    match = re.search(rf"const {re.escape(name)}[^=]*=\s*", source)
    if not match:
        raise KeyError(name)
    start = source.index("[", match.end())
    depth = 0
    j = start
    while True:
        if source[j] == '"':
            j += 1
            while source[j] != '"':
                j += 2 if source[j] == "\\" else 1
        elif source[j] == "[":
            depth += 1
        elif source[j] == "]":
            depth -= 1
            if depth == 0:
                break
        j += 1
    return parse_literals(source[start + 1 : j])


def labeled_arrays(source: str, open_marker: str):
    """Collect `"label" => &[ ... ]` arms (with `_ =>` mapped to "default"),
    starting from `open_marker`."""
    source = strip_comments(source)
    at = source.index(open_marker)
    out: dict[str, list] = {}
    for match in re.finditer(r'(?:"([^"]+)"|(_))\s*=>\s*&\[', source[at:]):
        label = match.group(1) or "default"
        open_at = at + match.end()
        depth = 1
        j = open_at
        while depth:
            if source[j] == '"':
                j += 1
                while source[j] != '"':
                    j += 2 if source[j] == "\\" else 1
            elif source[j] == "[":
                depth += 1
            elif source[j] == "]":
                depth -= 1
            j += 1
        out[label] = parse_literals(source[open_at : j - 1])
    return out


def write(name: str, payload: dict, preserve: tuple[str, ...] = ()) -> None:
    path = LEXICON_DIR / name
    if path.exists():
        existing = json.loads(path.read_text(encoding="utf-8"))
        for key in preserve:
            if key in existing and key not in payload:
                payload[key] = existing[key]
    payload = {"schema": "lexicon.v1", **payload}
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {path.relative_to(RUST_ROOT)}")


def main() -> int:
    stopwords_src = (RUST_ROOT / "visualner/src/extractor/stopwords.rs").read_text(encoding="utf-8")
    ranking_src = (RUST_ROOT / "visualner/src/extractor/ranking.rs").read_text(encoding="utf-8")
    extractor_src = (RUST_ROOT / "visualner/src/extractor.rs").read_text(encoding="utf-8")
    values_src = (RUST_ROOT / "visualner/src/extractor/values.rs").read_text(encoding="utf-8")
    scoring_src = (RUST_ROOT / "semantic-core/src/phrase_impact/scoring.rs").read_text(encoding="utf-8")
    causal_src = (RUST_ROOT / "pipelinegen-muscles/src/scene_intelligence/causal.rs").read_text(encoding="utf-8")
    comparison_src = (RUST_ROOT / "pipelinegen-muscles/src/scene_intelligence/comparison.rs").read_text(encoding="utf-8")

    # 1. stopwords.json — every curated candidate-exclusion vocabulary.
    stop_words = const_array(stopwords_src, "STOP_WORDS")
    # New temporal prepositions: "spalti dopo"/"lancio simulato durante..."
    # glued a connective to its noun ("spalti dopo" was extracted as one
    # entity). They belong to the same spatial/temporal class as the existing
    # "davanti"/"dietro" entries.
    stop_words += ["dopo", "durante"]
    write(
        "stopwords.json",
        {
            "stop_words": stop_words,
            "non_ascii_stop_words": const_array(stopwords_src, "NON_ASCII_STOP_WORDS"),
            "stop_phrases": const_array(stopwords_src, "STOP_PHRASES"),
            "title_words": const_array(stopwords_src, "TITLE_WORDS"),
            "generic_phrases": const_array(ranking_src, "GENERIC_PHRASES"),
        },
        # Data-only table added after the migration; never drop it on a rerun.
        preserve=("articulated_prepositions",),
    )

    # 2. object_vocabulary.json — the per-language visual-object lexicon.
    write("object_vocabulary.json", {"terms": labeled_arrays(extractor_src, "fn object_lexicon_candidates")})

    # 3. scale_words.json — scaled-quantity vocabulary.
    write("scale_words.json", {"words": const_array(values_src, "LOCAL_SCALE_WORDS")})

    # 4. discourse_cues.json — every discourse/salience cue used by phrase
    #    impact and scene intelligence, in one shared table.
    write(
        "discourse_cues.json",
        {
            "content_cues": labeled_arrays(scoring_src, "fn content_signal"),
            "filler_words": ["um", "uh", "erm", "er", "hmm", "ah"],
            "filler_phrases": [["you", "know"], ["i", "mean"]],
            "causal_after": const_array(causal_src, "CAUSE_AFTER_MARKERS"),
            "causal_before": const_array(causal_src, "CAUSE_BEFORE_MARKERS"),
            "hypothetical": const_array(causal_src, "HYPOTHETICAL_MARKERS"),
            "attribution": const_array(causal_src, "ATTRIBUTION_MARKERS"),
            "contribute": const_array(causal_src, "CONTRIBUTE_MARKERS"),
            "sequence": const_array(causal_src, "SEQUENCE_MARKERS"),
            "comparison_signals": const_array(comparison_src, "COMPARISON_SIGNALS"),
            "comparative_adj": const_array(comparison_src, "COMPARATIVE_ADJ"),
            "negation_tokens": const_array(comparison_src, "NEGATION_TOKENS"),
            "contrast_verbs": const_array(comparison_src, "CONTRAST_VERBS"),
        },
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
