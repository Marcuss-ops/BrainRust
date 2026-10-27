#!/usr/bin/env python3
"""Deterministic multilingual corpus smoke/quality benchmark for BrainRust."""
import argparse, json, os, pathlib, re, select, subprocess, tempfile, time
from collections import Counter
ROOT = pathlib.Path(__file__).resolve().parent

def worker_metrics(pid):
    """Read Linux process high-water memory and cumulative CPU counters."""
    try:
        status = pathlib.Path(f"/proc/{pid}/status").read_text(encoding="ascii")
        values = {}
        for line in status.splitlines():
            key, _, rest = line.partition(":")
            if key in {"VmRSS", "VmHWM", "VmSize"}:
                values[key] = int(rest.split()[0])
        stat = pathlib.Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
        fields = stat.rsplit(")", 1)[1].split()
        ticks = int(fields[11]) + int(fields[12])
        values["cpu_seconds"] = ticks / os.sysconf("SC_CLK_TCK")
        return values
    except (FileNotFoundError, IndexError, ValueError, OSError):
        return {}

def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, min(len(ordered) - 1, int((len(ordered) - 1) * fraction + 0.5)))]

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
    entities = []; dates=[]; charts=[]; bullet=[]; micro=[]; groups=[]; proposals=[]; contextual=[]
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
        for query in r.get("contextual_image_queries",[]):
            evidence=query.get("evidence","")
            start=query.get("evidence_start_byte"); end=query.get("evidence_end_byte")
            components=query.get("components",[])
            valid=(isinstance(start,int) and isinstance(end,int) and 0<=start<end<=len(raw)
                   and raw[start:end].decode("utf8",errors="replace")==evidence
                   and query.get("kind")=="contextual_image_query" and len(components)==2)
            component_texts=[]
            for component in components:
                a=component.get("start_byte"); b=component.get("end_byte"); surface=component.get("text","")
                grounded=(isinstance(start,int) and isinstance(end,int) and
                          isinstance(a,int) and isinstance(b,int) and start<=a<b<=end
                          and raw[a:b].decode("utf8",errors="replace")==surface)
                valid=valid and grounded and bool(component.get("component_type"))
                component_texts.append(surface)
            # Relation targets and same-sentence co-mentions are distinct
            # retrieval cues. Neither may invent components or an assertion.
            co_mention = all(c.get("kind") == "co_mentioned_entity" for c in components)
            relation = any(c.get("kind") in {"location", "related_entity"} for c in components)
            if co_mention:
                grounded_ids = []
                for component in components:
                    ids = {entity["id"] for entity in r.get("entities", [])
                           if entity.get("start_byte") == component.get("start_byte")
                           and entity.get("end_byte") == component.get("end_byte")
                           and entity.get("text") == component.get("text")}
                    grounded_ids.append(ids)
                supported = len(grounded_ids) == 2 and any(
                    mention.get("sentence_index") == query.get("sentence_index")
                    and mention.get("evidence") == evidence
                    and ((mention.get("left_id") in grounded_ids[0] and mention.get("right_id") in grounded_ids[1])
                         or (mention.get("right_id") in grounded_ids[0] and mention.get("left_id") in grounded_ids[1]))
                    for mention in r.get("relations", {}).get("co_mentions", []))
            else:
                component_ids = [
                    {entity["id"] for entity in r.get("entities", [])
                     if entity.get("start_byte") == component.get("start_byte")
                     and entity.get("end_byte") == component.get("end_byte")
                     and entity.get("text") == component.get("text")}
                    for component in components]
                supported = relation and len(component_ids) == 2 and any(
                    fact.get("status") == "asserted"
                    and fact.get("subject_entity_id") in component_ids[0]
                    and (fact.get("target_entity_id") in component_ids[1]
                         or (fact.get("predicate") == "located_in"
                             and components[1].get("text") == fact.get("value")
                             and components[1].get("start_byte") == fact.get("start_byte")
                             and components[1].get("end_byte") == fact.get("end_byte")))
                    and fact.get("sentence_index") == query.get("sentence_index")
                    and fact.get("evidence") == evidence
                    and fact.get("target_entity_id")
                    for fact in r.get("entity_facts", []))
            valid=valid and supported and query.get("query")==" ".join(component_texts)
            if not valid:
                problems.append(f"{scene_id}: contextual image query is unsupported or ungrounded")
                break
            contextual.append(query)
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
    expected_chart = sorted(float(value) for value in ea["percentages"])
    matching_charts = []
    for chart in charts:
        items = chart.get("chart_items", [])
        kinds = {item.get("kind") for item in items}
        values = sorted(float(item["value"]) for item in items if isinstance(item.get("value"), (int, float)))
        if len(items) == 2 and kinds == {"PERCENT"} and values == expected_chart:
            matching_charts.append(chart)
    if len(matching_charts) != 1:
        review.append(
            f"expected one two-value percent chart {expected_chart}; "
            f"found {len(matching_charts)} valid among {len(charts)} proposals"
        )
    for surface, expected_type in (
        (ea["organization"], "ORG"),
        (ea["city"], "LOCATION"),
    ):
        if not any(
            entity.get("type") == expected_type
            and surface.casefold() in str(entity.get("text", "")).casefold()
            for entity in entities
        ):
            review.append(f"missing typed {expected_type}: {surface}")
    expected_object_tokens = {
        token.casefold() for token in re.findall(r"[^\W_]+", ea["object"], flags=re.UNICODE)
    }
    # A composed source description can be represented by its typed object
    # head ("sensor" in "sensor de agua"). Keep that distinct from exact
    # phrase coverage so the report shows whether modifiers/context survived.
    exact_object = any(
        ea["object"].casefold() in str(entity.get("text", "")).casefold()
        and entity.get("type") in {"OBJECT", "VISUAL_CONCEPT"}
        for entity in entities
    )
    object_head = any(
        entity.get("type") == "OBJECT"
        and bool(
            expected_object_tokens
            & {
                token.casefold()
                for token in re.findall(
                    r"[^\W_]+", str(entity.get("text", "")), flags=re.UNICODE
                )
            }
        )
        for entity in entities
    )
    if not exact_object and not object_head:
        review.append("missing visual object: " + ea["object"])
    if not bullet: review.append("no bullet candidates")
    if not any(x.get("type")=="LOCATION" for x in entities):review.append("no LOCATION")
    if not any(x.get("proposal_type")=="entity_search" for x in proposals):review.append("no search suggestions")
    status="FAIL" if problems else ("NEEDS_REVIEW" if review else "PASS")
    return {"id":case["id"],"language":case["lang"],"topic":case["topic"],"status":status,
     "problems":problems[:12],"review":review[:12],"entities":len(entities),
     "visual_concepts":sum(e.get("type")=="VISUAL_CONCEPT" for e in entities),
     "dates":len(dates),"bullets":len(bullet),"micro_summaries":len(micro),
     "groups":len(groups),"charts":len(charts),"search_proposals":sum(x.get("proposal_type")=="entity_search" for x in proposals),
     "co_mentions":sum(len(s.get("relations",{}).get("co_mentions",[])) for s in records),
     "contextual_image_queries":len(contextual),
     "expected_object_exact_phrase":exact_object,"expected_object_typed_head":object_head}

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
    resource_samples=[]
    try:
        health=worker.request({"operation":"health"},args.timeout)
        if not health.get("ok"):raise RuntimeError("worker health failed")
        resource_samples.append(worker_metrics(worker.p.pid))
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
                    resource_samples.append(worker_metrics(worker.p.pid))
                    analysis_seconds=time.monotonic()-t0
                    if not response.get("ok"):
                        row={"id":c["id"],"language":c["lang"],"status":"FAIL","problems":[str(response.get("error"))]}
                    else:
                        row=inspect(c,response["manifest"],scenes)
                        replay_start=time.monotonic()
                        replay=worker.request(request,args.timeout)
                        replay_identical=(replay.get("manifest")==response["manifest"] and replay.get("cached"))
                        row["replay_seconds"]=round(time.monotonic()-replay_start,4)
                        row["replay_identical"]=bool(replay_identical)
                        if not replay_identical:
                            row["status"]="FAIL";row["problems"].append("cache replay mismatch")
                    row["seconds"]=round(analysis_seconds,2)
                except Exception as e:
                    row={"id":c["id"],"language":c["lang"],"status":"FAIL","problems":[str(e)]}
            result.append(row)
            print(f'{i+1:3d}/{len(cases)} {c["id"]:24} {row["status"]:12} chart={row.get("charts","-")} micro={row.get("micro_summaries","-")}',flush=True)
    finally:worker.close()
    latencies=[x["seconds"] for x in result if isinstance(x.get("seconds"),(int,float))]
    replay_latencies=[x["replay_seconds"] for x in result if isinstance(x.get("replay_seconds"),(int,float))]
    latest_metrics=next((sample for sample in reversed(resource_samples) if sample),{})
    total_words=sum(c.get("word_count",0) for c in cases)
    elapsed=round(time.monotonic()-start,2)
    output=args.report or str(pathlib.Path(tempfile.gettempdir())/"brainrust_multilingual_100_report.json")
    pathlib.Path(output).parent.mkdir(parents=True,exist_ok=True)
    report={"schema":"brainrust.multilingual100.v1","totals":dict(Counter(x["status"] for x in result)),
      "duration_seconds":elapsed,"cases":result,
      "performance":{"input_words":total_words,
       "throughput_words_per_second":round(total_words/elapsed,2) if elapsed else None,
       "latency_p50_seconds":percentile(latencies,0.50),
       "latency_p95_seconds":percentile(latencies,0.95),
       "cache_replay_count":sum("replay_identical" in row for row in result),
       "cache_replay_identical_count":sum(row.get("replay_identical") is True for row in result),
       "cache_replay_latency_p50_seconds":percentile(replay_latencies,0.50),
       "cache_replay_latency_p95_seconds":percentile(replay_latencies,0.95),
       "worker_peak_rss_kb":max((s.get("VmHWM",0) for s in resource_samples),default=0),
       "worker_peak_vmsize_kb":max((s.get("VmSize",0) for s in resource_samples),default=0),
       "worker_cpu_seconds":round(latest_metrics["cpu_seconds"],2) if "cpu_seconds" in latest_metrics else None,
       "worker_cpu_cores_average":round(latest_metrics["cpu_seconds"]/elapsed,3) if elapsed and "cpu_seconds" in latest_metrics else None}}
    pathlib.Path(output).write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print("RESULT:",report["totals"],"REPORT:",output)
    return 1 if any(x["status"]=="FAIL" for x in result) else (2 if any(x["status"]=="NEEDS_REVIEW" for x in result) else 0)
if __name__=="__main__":raise SystemExit(main())
