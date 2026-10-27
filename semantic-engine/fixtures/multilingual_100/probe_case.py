#!/usr/bin/env python3
"""Probe one fixture case and dump ORG / OBJECT / VISUAL_CONCEPT entities."""
import json, pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from benchmark import NDJSONWorker, make_scenes

ROOT = pathlib.Path(__file__).resolve().parent

def main():
    case_id = sys.argv[1]
    worker_path = sys.argv[2]
    cases = json.loads((ROOT / "cases.json").read_text(encoding="utf-8"))
    case = next(c for c in cases if c["id"] == case_id)
    text = (ROOT / case["path"]).read_text(encoding="utf-8")
    scenes = make_scenes(text, 220)
    worker = NDJSONWorker(worker_path)
    try:
        req = {"operation": "analyze_script", "language": case["lang"], "scenes": scenes}
        resp = worker.request(req, 180)
    finally:
        worker.close()
    if not resp.get("ok"):
        print("ERROR:", resp.get("error"))
        return
    manifest = resp["manifest"]
    entities = []
    for r in manifest.get("scenes", []):
        entities.extend(r.get("entities", []))
    print("expected:", json.dumps(case["expected_source_anchors"], ensure_ascii=False))
    print("total entities:", len(entities))
    for e in entities:
        t = e.get("type", "")
        if t in {"ORG", "OBJECT", "VISUAL_CONCEPT", "LOCATION", "PERSON"}:
            print(f"  {t:15} {e.get('text','')!r}")
    # Also dump any entity text containing hyphen
    print("hyphenated entity texts:")
    for e in entities:
        if "-" in str(e.get("text", "")):
            print(f"  {e.get('type','')} {e.get('text','')!r}")

if __name__ == "__main__":
    main()
