#!/usr/bin/env python3
"""Unit tests for the long-gold editorial quality gate."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("score_long_gold.py")
SPEC = importlib.util.spec_from_file_location("score_long_gold", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
score = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(score)


def report(
    *,
    entities: float | None = 1.0,
    surface: float | None = 1.0,
    typed: float | None = 1.0,
    grounding: int = 0,
    replay: bool = True,
) -> dict:
    return {
        "overall": {
            "named_entities": {"recall": entities},
            "visual_objects_surface_only": {"recall": surface},
            "visual_objects_typed": {"recall": typed},
            "grounding_errors": grounding,
            "replay_identical": replay,
        }
    }


class QualityGateTest(unittest.TestCase):
    def test_passes_at_targets(self) -> None:
        self.assertEqual(score.quality_gate_failures(report()), [])

    def test_reports_each_target_failure(self) -> None:
        failures = score.quality_gate_failures(
            report(entities=0.5, surface=0.8, typed=0.6, grounding=2, replay=False)
        )
        self.assertEqual(len(failures), 5)
        self.assertTrue(any("entity recall: 0.500 < 0.950" in item for item in failures))
        self.assertTrue(any("surface object recall: 0.800 < 0.900" in item for item in failures))
        self.assertTrue(any("typed object recall: 0.600 < 0.900" in item for item in failures))
        self.assertTrue(any("grounding errors: 2 != 0" in item for item in failures))
        self.assertTrue(any("replay" in item for item in failures))

    def test_missing_measurement_is_a_failure(self) -> None:
        failures = score.quality_gate_failures(report(typed=None))
        self.assertEqual(len(failures), 1)
        self.assertIn("typed object recall: not measured", failures[0])


if __name__ == "__main__":
    unittest.main()
