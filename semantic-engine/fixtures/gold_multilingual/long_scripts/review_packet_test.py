from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import review_packet

BASE = Path(__file__).resolve().parent


class ReviewPacketTest(unittest.TestCase):
    def test_packet_contains_all_drafts_without_promotion(self) -> None:
        gold_path = BASE / "gold.json"
        before = gold_path.read_bytes()
        gold = json.loads(before)

        packet = review_packet.build_packet(gold_path)

        self.assertEqual(19, packet["draft_count"])
        self.assertEqual(review_packet.DRAFT_AUTHORITY, packet["draft_authority"])
        ids = {draft["id"] for draft in packet["drafts"]}
        expected = {
            script_id
            for script_id, spec in gold["scripts"].items()
            if spec.get("authority") == review_packet.DRAFT_AUTHORITY
        }
        self.assertEqual(expected, ids)
        self.assertNotIn("it_stadium_long_01", ids)
        self.assertTrue(all(draft["authority"] == review_packet.DRAFT_AUTHORITY for draft in packet["drafts"]))
        self.assertTrue(all(draft["authority_remains"] == review_packet.DRAFT_AUTHORITY for draft in packet["drafts"]))
        self.assertTrue(all(draft["review_status"] == "pending_human_review" for draft in packet["drafts"]))
        self.assertTrue(all(draft["source_text"].strip() for draft in packet["drafts"]))
        self.assertNotIn("human_annotator", json.dumps(packet["drafts"], ensure_ascii=False))
        self.assertEqual(before, gold_path.read_bytes(), "packet generation must not mutate gold.json")

    def test_markdown_keeps_every_draft_pending(self) -> None:
        packet = review_packet.build_packet(BASE / "gold.json")
        markdown = review_packet.render_markdown(packet)

        self.assertIn("# TICKET-08 draft review packet", markdown)
        self.assertIn("**19**", markdown)
        self.assertIn(review_packet.DRAFT_AUTHORITY, markdown)
        self.assertNotIn("human_annotator", markdown)
        self.assertIn("## `de_stadion_long_01`", markdown)
        self.assertIn("## `ru_stadion_long_02`", markdown)

    def test_missing_source_fails_closed(self) -> None:
        gold = json.loads((BASE / "gold.json").read_text(encoding="utf-8"))
        script_id = next(
            sid
            for sid, spec in gold["scripts"].items()
            if spec.get("authority") == review_packet.DRAFT_AUTHORITY
        )
        gold["scripts"][script_id]["file"] = "missing.txt"
        with tempfile.TemporaryDirectory() as tmp:
            gold_path = Path(tmp) / "gold.json"
            gold_path.write_text(json.dumps(gold), encoding="utf-8")
            with self.assertRaises(review_packet.ReviewPacketError):
                review_packet.build_packet(gold_path)

    def test_path_escape_is_rejected(self) -> None:
        gold = json.loads((BASE / "gold.json").read_text(encoding="utf-8"))
        script_id = next(
            sid
            for sid, spec in gold["scripts"].items()
            if spec.get("authority") == review_packet.DRAFT_AUTHORITY
        )
        gold["scripts"][script_id]["file"] = "../../../outside.txt"
        with tempfile.TemporaryDirectory() as tmp:
            gold_path = Path(tmp) / "gold.json"
            gold_path.write_text(json.dumps(gold), encoding="utf-8")
            with self.assertRaises(review_packet.ReviewPacketError):
                review_packet.build_packet(gold_path)


if __name__ == "__main__":
    unittest.main()
