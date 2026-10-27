import copy
import unittest

from benchmark import inspect


class ContextualQueryChecks(unittest.TestCase):
    def setUp(self):
        self.text = "Alice and Berlin."
        self.case = {"id": "test", "lang": "en", "topic": "test", "expected_source_anchors": {
            "person": "Alice", "secondary_person": "Alice", "years": [],
            "percentages": [42, 18], "organization": "Council", "city": "Berlin", "object": "sensor",
        }}
        self.scene = {"scene_id": "s", "entities": [
            {"id": "a", "type": "PERSON", "text": "Alice", "start_byte": 0, "end_byte": 5},
            {"id": "b", "type": "LOCATION", "text": "Berlin", "start_byte": 10, "end_byte": 16},
        ], "relations": {"co_mentions": [{"left_id": "a", "right_id": "b", "sentence_index": 0, "evidence": self.text}]},
            "contextual_image_queries": [{"kind": "contextual_image_query", "query": "Alice Berlin",
                "sentence_index": 0, "evidence": self.text, "evidence_start_byte": 0, "evidence_end_byte": len(self.text),
                "components": [
                    {"kind": "co_mentioned_entity", "component_type": "PERSON", "text": "Alice", "start_byte": 0, "end_byte": 5},
                    {"kind": "co_mentioned_entity", "component_type": "LOCATION", "text": "Berlin", "start_byte": 10, "end_byte": 16},
                ]}]}

    def problems(self, scene):
        return inspect(self.case, {"schema_version": "semantic_manifest.v2", "scenes": [scene]},
                       [{"scene_id": "s", "text": self.text}])["problems"]

    def test_grounded_co_mention_is_not_required_to_assert_location(self):
        self.assertEqual(self.problems(self.scene), [])

    def test_asserted_relation_requires_matching_subject_and_target(self):
        scene = copy.deepcopy(self.scene)
        components = scene["contextual_image_queries"][0]["components"]
        components[0]["kind"] = "entity"
        components[1]["kind"] = "location"
        scene["entity_facts"] = [{"subject_entity_id": "a", "target_entity_id": "b",
            "predicate": "located_in", "status": "asserted", "sentence_index": 0,
            "evidence": self.text, "evidence_start_byte": 0, "evidence_end_byte": len(self.text)}]
        self.assertEqual(self.problems(scene), [])
        for field, value in (("status", "negated"), ("subject_entity_id", "invented"),
                             ("target_entity_id", "invented")):
            with self.subTest(field=field):
                corrupt = copy.deepcopy(scene)
                corrupt["entity_facts"][0][field] = value
                self.assertTrue(self.problems(corrupt))

    def test_missing_relation_or_corrupted_evidence_fails(self):
        for corruption in ("relation", "span", "query", "entity", "component_count"):
            with self.subTest(corruption=corruption):
                scene = copy.deepcopy(self.scene)
                query = scene["contextual_image_queries"][0]
                if corruption == "relation":
                    scene["relations"]["co_mentions"] = []
                elif corruption == "span":
                    query["components"][1]["end_byte"] = 15
                elif corruption == "query":
                    query["query"] = "Alice Paris"
                elif corruption == "entity":
                    scene["entities"][1]["id"] = "invented"
                else:
                    query["components"] = []
                self.assertTrue(self.problems(scene))


if __name__ == "__main__":
    unittest.main()
