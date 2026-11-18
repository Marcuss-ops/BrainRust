#!/usr/bin/env python3
"""Export the TICKET-08 drafts for human review without promoting them.

The packet is deliberately read-only with respect to `gold.json`. It lists
every script whose authority is exactly `assistant_draft_pending_human_review`,
puts the source text next to the current machine annotations, and gives the
reviewer a checklist. It never writes `human_annotator`, never adds an approval
field, and never closes TICKET-08: the independent held-out annotation still
requires a human owner.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

BASE = Path(__file__).resolve().parent
PACKET_SCHEMA = "brainrust.ticket08.review_packet.v1"
DRAFT_AUTHORITY = "assistant_draft_pending_human_review"

ANNOTATION_FIELDS = (
    "entities",
    "visual_objects",
    "important_phrases",
    "important_segments",
    "facts",
    "positive_charts",
    "negative_chart_evidence",
    "sequence",
    "contextual_image_queries",
    "micro_summary_expectations",
    "critical_invariants",
    "noise",
    "capability_gap_expectations",
)

REVIEW_CHECKLIST = (
    "Every entity or visual object text appears verbatim in the source text.",
    "Every entity type is supported by the source, not guessed from the name.",
    "Every fact preserves subject, predicate, value, polarity and date.",
    "Every important phrase is an exact source sentence or a complete adjacent sentence span.",
    "Every positive chart value is explicitly present in the evidence and is not a prediction presented as an achieved result.",
    "Every negative chart evidence item really is non-chart evidence.",
    "Every sequence step appears in the source in the listed order.",
    "Every contextual image query is answerable from its cited evidence.",
    "No claim or invariant is promoted beyond what the source explicitly supports.",
    "The draft authority is left unchanged and the review decision is recorded in the review system, not in this packet.",
)


class ReviewPacketError(ValueError):
    """Raised when the gold set or a referenced source file is not reviewable."""


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ReviewPacketError(f"cannot read {path}: {exc}") from exc


def _source_text(gold_dir: Path, spec: dict[str, Any], script_id: str) -> tuple[str, str]:
    filename = spec.get("file")
    if not isinstance(filename, str) or not filename.strip():
        raise ReviewPacketError(f"{script_id} has no source file")
    path = (gold_dir / filename).resolve()
    if path.parent != gold_dir.resolve():
        raise ReviewPacketError(f"{script_id} source escapes the gold directory: {filename!r}")
    try:
        return filename, path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ReviewPacketError(f"{script_id} source file is unreadable: {filename}: {exc}") from exc


def build_packet(gold_path: Path) -> dict[str, Any]:
    """Return the review packet. The input file is only read, never written."""
    gold_path = gold_path.resolve()
    gold = load_json(gold_path)
    if not isinstance(gold, dict):
        raise ReviewPacketError("gold must be an object")
    scripts = gold.get("scripts")
    if not isinstance(scripts, dict) or not scripts:
        raise ReviewPacketError("gold.scripts must be a non-empty object")

    drafts: list[dict[str, Any]] = []
    excluded = 0
    for script_id in sorted(scripts):
        spec = scripts[script_id]
        if not isinstance(spec, dict):
            raise ReviewPacketError(f"gold.scripts[{script_id}] must be an object")
        if spec.get("authority") != DRAFT_AUTHORITY:
            excluded += 1
            continue

        source_file, source_text = _source_text(gold_path.parent, spec, script_id)
        annotations = {
            field: spec[field]
            for field in ANNOTATION_FIELDS
            if field in spec
        }
        drafts.append(
            {
                "id": script_id,
                "language": spec.get("language"),
                "source_file": source_file,
                "source_sha256": hashlib.sha256(source_text.encode("utf-8")).hexdigest(),
                "authority": spec["authority"],
                "authority_remains": DRAFT_AUTHORITY,
                "review_status": "pending_human_review",
                "source_text": source_text,
                "annotations": annotations,
                "review_checklist": list(REVIEW_CHECKLIST),
            }
        )

    if not drafts:
        raise ReviewPacketError(f"no drafts with authority {DRAFT_AUTHORITY!r} found")

    return {
        "schema": PACKET_SCHEMA,
        "ticket": "TICKET-08",
        "source": gold_path.name,
        "draft_count": len(drafts),
        "excluded_non_draft_count": excluded,
        "draft_authority": DRAFT_AUTHORITY,
        "packet_effect": "none: this export does not change gold.json or promote any draft",
        "warning": (
            "This packet reviews existing machine drafts. It is not an independent "
            "held-out annotation set and it does not close TICKET-08."
        ),
        "drafts": drafts,
    }


def render_markdown(packet: dict[str, Any]) -> str:
    lines = [
        "# TICKET-08 draft review packet",
        "",
        f"- Schema: `{packet['schema']}`",
        f"- Source: `{packet['source']}`",
        f"- Drafts: **{packet['draft_count']}**",
        f"- Authority of every draft: `{packet['draft_authority']}`",
        f"- Packet effect: {packet['packet_effect']}",
        "",
        f"> {packet['warning']}",
        "",
        "This packet is a read-only export. Do not copy an approval or promote a draft "
        "by editing this file; record review decisions in the review system.",
        "",
        "## Draft index",
        "",
        "| Script | Language | Source | Entities | Objects | Phrases | Facts |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for draft in packet["drafts"]:
        annotations = draft["annotations"]
        lines.append(
            "| `{id}` | {language} | `{source_file}` | {entities} | {objects} | {phrases} | {facts} |".format(
                id=draft["id"],
                language=draft["language"],
                source_file=draft["source_file"],
                entities=len(annotations.get("entities") or []),
                objects=len(annotations.get("visual_objects") or []),
                phrases=len(annotations.get("important_phrases") or []),
                facts=len(annotations.get("facts") or []),
            )
        )

    for draft in packet["drafts"]:
        lines.extend(
            [
                "",
                f"## `{draft['id']}`",
                "",
                f"- Language: `{draft['language']}`",
                f"- Source: `{draft['source_file']}`",
                f"- Source SHA-256: `{draft['source_sha256']}`",
                f"- Authority: `{draft['authority']}` (unchanged)",
                f"- Review status: `{draft['review_status']}`",
                "",
                "### Source text",
                "",
                "```text",
                draft["source_text"].rstrip("\n"),
                "```",
                "",
                "### Machine draft annotations",
                "",
                "```json",
                json.dumps(draft["annotations"], indent=2, ensure_ascii=False),
                "```",
                "",
                "### Review checklist",
                "",
            ]
        )
        lines.extend(f"- [ ] {item}" for item in draft["review_checklist"])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gold", type=Path, default=BASE / "gold.json",
                        help="long-gold gold.json (default: %(default)s)")
    parser.add_argument("--output-dir", type=Path, default=BASE / "review_packet",
                        help="directory for review_packet.json and review_packet.md")
    args = parser.parse_args()

    try:
        packet = build_packet(args.gold)
    except ReviewPacketError as exc:
        print(f"review-packet: {exc}", file=sys.stderr)
        return 2

    args.output_dir.mkdir(parents=True, exist_ok=True)
    json_path = args.output_dir / "review_packet.json"
    markdown_path = args.output_dir / "review_packet.md"
    json_path.write_text(json.dumps(packet, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    markdown_path.write_text(render_markdown(packet), encoding="utf-8")
    print(
        "review-packet:",
        f"drafts={packet['draft_count']}",
        f"authority={packet['draft_authority']}",
        f"json={json_path}",
        f"markdown={markdown_path}",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
