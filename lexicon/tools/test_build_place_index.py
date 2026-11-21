"""Validation tests for the frozen country/capital place seed."""
import json
import unittest
from pathlib import Path

import build_place_index as place_builder

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "lexicon/source/wikidata_country_capitals_20261121.json"
OUTPUT = ROOT / "lexicon/place_index_v1.json"


class PlaceIndexTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = json.loads(SOURCE.read_text(encoding="utf-8"))
        cls.index = place_builder.build(cls.source)

    def test_checked_in_index_is_reproducible_from_frozen_source(self):
        checked_in = json.loads(OUTPUT.read_text(encoding="utf-8"))
        self.assertEqual(self.index, checked_in)
        self.assertEqual(sum(row["kind"] == "country" for row in self.index["entries"]), 195)
        self.assertGreaterEqual(sum(row["kind"] == "city" for row in self.index["entries"]), 190)

    def test_parent_ids_resolve_and_collisions_are_explicit(self):
        ids = {row["id"] for row in self.index["entries"]}
        self.assertTrue(all(set(row["parent_ids"]) <= ids for row in self.index["entries"]))
        collisions = self.index["alias_collisions"]
        self.assertTrue(all(len(row["ids"]) > 1 for row in collisions))
        self.assertTrue(all(set(row["ids"]) <= ids for row in collisions))

    def test_every_supported_language_has_a_frozen_localized_alias(self):
        by_id = {row["id"]: row for row in self.index["entries"]}
        germany = by_id["wd:Q183"]["aliases"]
        self.assertEqual(set(germany), set(place_builder.LANGUAGES))
        self.assertEqual(germany["de"], ["Deutschland"])
        self.assertEqual(germany["es"], ["Alemania"])
        self.assertEqual(germany["ru"], ["Германия"])
        self.assertEqual(by_id["wd:Q270"]["aliases"]["pl"], ["Warszawa"])
        self.assertEqual(by_id["wd:Q649"]["aliases"]["ru"], ["Москва"])
        self.assertEqual(by_id["wd:Q3640"]["aliases"]["tr"], ["Ankara"])
        self.assertEqual(by_id["wd:Q585"]["aliases"]["ru"], ["Осло"])
        self.assertEqual(by_id["wd:Q3787"]["aliases"]["de"], ["Abuja"])
        self.assertTrue(all(
            set(place_builder.LANGUAGES) <= set(row["aliases"])
            for row in self.index["entries"]
        ))

    def test_coordinate_bounds_are_enforced(self):
        bad = json.loads(json.dumps(self.index))
        bad["entries"][0]["lat"] = 91
        with self.assertRaisesRegex(ValueError, "latitude out of range"):
            place_builder.validate(bad)

    def test_source_checksum_is_enforced(self):
        bad = json.loads(json.dumps(self.source))
        bad["results"].pop()
        with self.assertRaisesRegex(ValueError, "checksum"):
            place_builder.build(bad)


if __name__ == "__main__":
    unittest.main()
