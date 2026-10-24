#!/usr/bin/env python3
"""Deterministic multilingual corpus smoke/quality benchmark for BrainRust."""
import argparse, json, os, pathlib, select, subprocess, tempfile, time
from collections import Counter
ROOT = pathlib.Path(__file__).resolve().parent

def make_scenes(text, budget):
    scenes, current, n = [], [], 0
    for block in text.strip().split("\n\n"):
        block = block.strip()
        if not block:
            continue
        words = len(block.split())
        if current and n + words > budget:
            scenes.append("\n\n".join(current))
            current, n = [], 0
        current.append(block)
        n += words
    if current: scenes.append("\n\n".join(current))
    return [{"scene_id": "s%02d" % (i+1), "text": part} for i,part in enumerate(scenes)]

class NDJSONWorker:
    def __init__(self, filename):
        self.errlog = tempfile.TemporaryFile()
        self.p = subprocess.Popen([filename],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.errlog)
        self.buffer = b""
    def request(self, payload, timeout):
        self.p.stdin.write((json.dumps(payload,ensure_ascii=False,separators=(",",":"))+"\n").encode("utf-8"))
        self.p.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            i = self.buffer.find(b"\n")
            if i >= 0:
                result = self.buffer[:i]
                self.buffer = self.buffer[i+1:]
                return json.loads(result)
            rem = deadline-time.monotonic()
            if rem <= 0 or self.p.poll() is not None:
                raise RuntimeError("worker exited or request deadline exceeded")
            ready,_,_ = select.select([self.p.stdout],[],[],rem)
            if not ready: raise TimeoutError("worker request timeout")
            chunk = os.read(self.p.stdout.fileno(), 1048576)
            if not chunk: raise RuntimeError("worker output EOF")
            self.buffer += chunk
    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=3)
        except Exception:
            self.p.kill()
            self.p.wait()
        self.errlog.close()

def inspect(case, manifest, scenes):
    problems, review = [], []
    byid = {s["scene_id"]:s["text"] for s in scenes}
    records = manifest.get("scenes",[])
    if manifest.get("schema_version")!="semantic_manifest.v2": problems.append("schema mismatch")
    if len(records)!=len(scenes): problems.append("scene count mismatch")
    entities = []; dates=[]; charts=[]; bullet=[]; micro=[]; groups=[]; proposals=[]
    for r in records:
        scene_id = r.get("scene_id")
        if scene_id not in byid:
            problems.append("unexpected scene");continue
        src=byid[scene_id]; raw=src.encode("utf-8")
        for key, attr in [("entities","text"),("bullets","text"),("short_phrases","text"),("long_phrases","text"),("important_words","text"),("quotes","text")]:
            for item in r.get(key,[]):
                a,b=item.get("start_byte"),item.get("end_byte")
                if not isinstance(a,int) or not isinstance(b,int) or not (0<=a<b<=len(raw)) or raw[a:b].decode("utf8",errors="replace")!=item.get(attr):
                    problems.append(f"{scene_id}.{key}: bad offset/source span");break
        for x in r.get("entity_facts",[]):
            a,b=x.get("evidence_start_byte"),x.get("evidence_end_byte")
            if not isinstance(a,int) or not isinstance(b,int) or not (0<=a<b<=len(raw)) or raw[a:b].decode("utf8",errors="replace")!=x.get("evidence"):
                problems.append(f"{scene_id}.entity_fact: evidence not grounded");break
        for x in r.get("visual_proposals",[]):
            if any(q not in src for q in x.get("visual_query_suggestions",[])):
                problems.append(f"{scene_id}: visual query not in source");break
        entities.extend(r.get("entities",[]))
        dates.extend(r.get("temporal_facts",[]))
        bullet.extend(r.get("bullets",[]))
        micro.extend(r.get("micro_summaries",[]))
        groups.extend(r.get("semantic_groups",[]))
        proposals.extend(r.get("visual_proposals",[]))
        charts.extend(x for x in r.get("visual_proposals",[]) if x.get("proposal_type")=="chart")
    ea=case["expected_source_anchors"]
    found_names={str(x.get("text","")).casefold() for x in entities if x.get("type")=="PERSON"}
    for person in [ea["person"],ea["secondary_person"]]:
        if person.casefold() not in found_names: review.append("missing person: "+person)
    normalized={str(x.get("normalized") or x.get("normalized_date") or "")[:4] for x in dates}
    for year in ea["years"]:
        if year not in normalized: review.append("missing normalized year: "+year)
    if not charts: review.append("explicit two-value percent chart missing")
    if not bullet: review.append("no bullet candidates")
    if not any(x.get("type")=="LOCATION" for x in entities):review.append("no LOCATION")
    if not any(x.get("proposal_type")=="entity_search" for x in proposals):review.append("no search suggestions")
    status="FAIL" if problems else ("NEEDS_REVIEW" if review else "PASS")
    return {"id":case["id"],"language":case["lang"],"topic":case["topic"],"status":status,
     "problems":problems[:12],"review":review[:12],"entities":len(entities),
     "visual_concepts":sum(e.get("type")=="VISUAL_CONCEPT" for e in entities),
     "dates":len(dates),"bullets":len(bullet),"micro_summaries":len(micro),
     "groups":len(groups),"charts":len(charts),"search_proposals":sum(x.get("proposal_type")=="entity_search" for x in proposals),
     "co_mentions":sum(len(s.get("relations",{}).get("co_mentions",[])) for s in records)}

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--worker",required=True)
    ap.add_argument("--language")
    ap.add_argument("--limit",type=int,default=0)
    ap.add_argument("--scene-words",type=int,default=220)
    ap.add_argument("--timeout",type=float,default=180)
    ap.add_argument("--report",default="")
    args=ap.parse_args()
    cases=json.loads((ROOT/"cases.json").read_text(encoding="utf-8"))
    if args.language: cases=[x for x in cases if x["lang"]==args.language]
    if args.limit:cases=cases[:args.limit]
    if not cases:ap.error("no test cases")
    worker=NDJSONWorker(args.worker); result=[]; start=time.monotonic()
    try:
        health=worker.request({"operation":"health"},args.timeout)
        if not health.get("ok"):raise RuntimeError("worker health failed")
        for i,c in enumerate(cases):
            text=(ROOT/c["path"]).read_text(encoding="utf-8")
            if len(text.split())!=c["word_count"] or len(text.encode("utf-8"))!=c["byte_count"]:
                row={"id":c["id"],"language":c["lang"],"status":"FAIL","problems":["fixture changed"]}
            else:
                scenes=make_scenes(text,args.scene_words)
                request={"operation":"analyze_script","language":c["lang"],"scenes":scenes}
                t0=time.monotonic()
                try:
                    response=worker.request(request,args.timeout)
                    if not response.get("ok"):
                        row={"id":c["id"],"language":c["lang"],"status":"FAIL","problems":[str(response.get("error"))]}
                    else:
                        row=inspect(c,response["manifest"],scenes)
                        if i==0:
                            replay=worker.request(request,args.timeout)
                            if replay.get("manifest")!=response["manifest"] or not replay.get("cached"):
                                row["status"]="FAIL";row["problems"].append("cache replay mismatch")
                    row["seconds"]=round(time.monotonic()-t0,2)
                except Exception as e:
                    row={"id":c["id"],"language":c["lang"],"status":"FAIL","problems":[str(e)]}
            result.append(row)
            print(f'{i+1:3d}/{len(cases)} {c["id"]:24} {row["status"]:12} chart={row.get("charts","-")} micro={row.get("micro_summaries","-")}',flush=True)
    finally:worker.close()
    output=args.report or str(pathlib.Path(tempfile.gettempdir())/"brainrust_multilingual_100_report.json")
    pathlib.Path(output).parent.mkdir(parents=True,exist_ok=True)
    report={"schema":"brainrust.multilingual100.v1","totals":dict(Counter(x["status"] for x in result)),
      "duration_seconds":round(time.monotonic()-start,2),"cases":result}
    pathlib.Path(output).write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print("RESULT:",report["totals"],"REPORT:",output)
    return 1 if any(x["status"]=="FAIL" for x in result) else (2 if any(x["status"]=="NEEDS_REVIEW" for x in result) else 0)
if __name__=="__main__":raise SystemExit(main())
