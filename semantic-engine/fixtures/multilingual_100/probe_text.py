#!/usr/bin/env python3
"""Probe extraction for inline text: probe_text.py <lang> <worker> <text>"""
import json, pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from benchmark import NDJSONWorker

def main():
    lang, worker_path, text = sys.argv[1], sys.argv[2], sys.argv[3]
    worker = NDJSONWorker(worker_path)
    try:
        req = {"operation": "analyze_script", "language": lang,
               "scenes": [{"scene_id": "s01", "text": text}]}
        resp = worker.request(req, 60)
    finally:
        worker.close()
    if not resp.get("ok"):
        print("ERROR:", resp.get("error"))
        return
    for r in resp["manifest"].get("scenes", []):
        for e in r.get("entities", []):
            print(f"  {e.get('type','?'):15} {e.get('text','')!r}")

if __name__ == "__main__":
    main()
