//! Persistent Scene Intelligence stdio worker (P0.1 + P0.2).
//!
//! One process, many analyses: newline-delimited JSON on stdin, one JSON
//! object per line on stdout — the same contract shape as
//! `phrase_impact_worker`. Batching N scenes into ONE request removes the
//! N-spawn cost the Go bridge previously paid (shadow mode had no process
//! wiring at all: Rust-produced JSON was consumed from fixtures).
//!
//! Operations (selected by `"operation"`, default `analyze`):
//! - `analyze`: `{scenes:[{scene_id,text,language,video_start_ms,
//!   video_end_ms}], entities_per_scene:[[..]]}` → `{ok:true, manifest}`.
//!   Scenes are sorted by id inside `analyze`, so request order never
//!   affects the manifest.
//! - `split_sentences` (P0.2): `{text, language?, scene_id?}` → `{ok:true, sentences:
//!   `[{text,start_byte,end_byte,scene_id}]` using the canonical
//!   language-aware splitter (byte offsets preserved, UTF-8 boundaries).
//!   `language` defaults to `en`; unlike phrase-impact embeddings, SI retains
//!   whole sentences instead of subdividing long spans.
//! - `preflight` (P1.5): `{certified_audio_ms, scenes:[{scene_id,
//!   duration_seconds, media_locator}]}` → `{ok:true}` or `{ok:false,
//!   error}`.
//! - `fingerprint` (P1.4): `{plan:{...}, strip_timing_state?}` →
//!   `{ok:true, fingerprint}` (FNV-1a 64 hex over canonical JSON).

use crate::overlay_fingerprint;
use crate::preflight::PreflightScene;
use crate::scene_intelligence::{self, SceneText};
use std::io;

/// Volatile keys the worker strips before fingerprinting when
/// `strip_timing_state` is true (default): wall-clock / timing state must
/// never change the cache identity of an identical plan.
const VOLATILE_KEYS: &[&str] = &["timing_state", "rendered_at_ms", "attempt_id"];

fn strip_volatile(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in VOLATILE_KEYS {
                map.remove(*key);
            }
            for (_, child) in map.iter_mut() {
                strip_volatile(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip_volatile(item);
            }
        }
        _ => {}
    }
}

pub fn process_line(line: &str) -> String {
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return json_error(format!("invalid scene-intelligence request JSON: {error}"));
        }
    };
    let operation = value
        .get("operation")
        .and_then(|op| op.as_str())
        .unwrap_or("analyze");
    match operation {
        "analyze" => process_analyze(&value),
        "split_sentences" => process_split(&value),
        "preflight" => process_preflight(&value),
        "fingerprint" => process_fingerprint(&value),
        unknown => json_error(format!("unknown scene-intelligence operation: {unknown}")),
    }
}

fn json_error(message: String) -> String {
    serde_json::json!({"ok": false, "error": message}).to_string()
}

fn process_analyze(value: &serde_json::Value) -> String {
    let scenes: Vec<SceneText> = value
        .get("scenes")
        .cloned()
        .and_then(|raw| serde_json::from_value(raw).ok())
        .unwrap_or_default();
    if scenes.is_empty() {
        return json_error("scene intelligence request requires scenes".to_string());
    }
    for scene in &scenes {
        if scene.scene_id.trim().is_empty() || scene.text.trim().is_empty() {
            return json_error("scene intelligence request requires scene id and text".to_string());
        }
        if scene.video_end_ms <= scene.video_start_ms {
            return json_error(
                "scene intelligence request requires a valid video window".to_string(),
            );
        }
    }
    let entities_per_scene: Vec<Vec<String>> = value
        .get("entities_per_scene")
        .cloned()
        .and_then(|raw| serde_json::from_value(raw).ok())
        .unwrap_or_else(|| vec![Vec::new(); scenes.len()]);
    if entities_per_scene.len() != scenes.len() {
        return json_error(format!(
            "entities_per_scene length {} does not match scenes length {}",
            entities_per_scene.len(),
            scenes.len()
        ));
    }
    // Canonical input order: sentence IDs are assigned in input order by
    // SharedContext::build, so sort scenes (with their entities) by id
    // first — otherwise two requests with the same scenes in different
    // order produce different evidence_sentence_ids and different hashes.
    let mut paired: Vec<(SceneText, Vec<String>)> =
        scenes.into_iter().zip(entities_per_scene).collect();
    paired.sort_by(|a, b| a.0.scene_id.cmp(&b.0.scene_id));
    let scenes: Vec<SceneText> = paired.iter().map(|(s, _)| s.clone()).collect();
    let entities_per_scene: Vec<Vec<String>> = paired.into_iter().map(|(_, e)| e).collect();
    let mut manifest = scene_intelligence::analyze(&scenes, &entities_per_scene);
    if let Err(error) = scene_intelligence::validate_manifest(&manifest) {
        return json_error(format!("scene intelligence manifest invalid: {error}"));
    }
    if let Err(error) = make_evidence_indices_scene_local(&mut manifest, &scenes) {
        return json_error(format!(
            "scene intelligence evidence indices invalid: {error}"
        ));
    }
    serde_json::json!({"ok": true, "manifest": manifest}).to_string()
}

// Detector internals use globally unique sentence IDs to share one context.
// The public manifest is per-scene, however, so export local sentence indices
// that consumers can validate against the owning scene text.
fn make_evidence_indices_scene_local(
    manifest: &mut scene_intelligence::SceneIntelligenceManifest,
    scenes: &[SceneText],
) -> Result<(), String> {
    let mut base = 0usize;
    for scene in scenes {
        let count = scene_intelligence::split_sentences(&scene.text, &scene.language).len();
        let end = base + count;
        let Some(signals) = manifest
            .scenes
            .iter_mut()
            .find(|signals| signals.scene_id == scene.scene_id)
        else {
            return Err(format!("missing scene {}", scene.scene_id));
        };
        let localize = |ids: &mut Vec<usize>| -> Result<(), String> {
            for id in ids.iter_mut() {
                if *id < base || *id >= end {
                    return Err(format!(
                        "scene {} cites sentence {} outside [{base}, {end})",
                        scene.scene_id, *id
                    ));
                }
                *id -= base;
            }
            Ok(())
        };
        for comparison in &mut signals.comparisons {
            localize(&mut comparison.evidence_sentence_ids)?;
        }
        for relation in &mut signals.causal {
            localize(&mut relation.evidence_sentence_ids)?;
        }
        for event in &mut signals.temporal_events {
            if event.narration_index < base || event.narration_index >= end {
                return Err(format!(
                    "scene {} narration index {} outside [{base}, {end})",
                    scene.scene_id, event.narration_index
                ));
            }
            event.narration_index -= base;
        }
        for relation in &mut signals.relations {
            localize(&mut relation.evidence_sentence_ids)?;
        }
        base = end;
    }
    Ok(())
}

fn process_split(value: &serde_json::Value) -> String {
    let text = value.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let scene_id = value
        .get("scene_id")
        .and_then(|s| s.as_str())
        .unwrap_or("scene-00");
    let language = value
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("en");
    let sentences = scene_intelligence::split_sentences(text, language);
    let items: Vec<serde_json::Value> = sentences
        .into_iter()
        .map(|segment| {
            serde_json::json!({
                "text": segment.text,
                "start_byte": segment.start_byte,
                "end_byte": segment.end_byte,
                "scene_id": scene_id,
            })
        })
        .collect();
    serde_json::json!({"ok": true, "sentences": items}).to_string()
}

fn process_preflight(value: &serde_json::Value) -> String {
    let certified_audio_ms = value
        .get("certified_audio_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let scenes: Vec<PreflightScene> = value
        .get("scenes")
        .cloned()
        .and_then(|raw| serde_json::from_value(raw).ok())
        .unwrap_or_default();
    match crate::preflight::validate(certified_audio_ms, &scenes) {
        Ok(()) => serde_json::json!({"ok": true}).to_string(),
        Err(error) => json_error(error.to_string()),
    }
}

fn process_fingerprint(value: &serde_json::Value) -> String {
    let Some(plan) = value.get("plan").cloned() else {
        return json_error("fingerprint request requires plan".to_string());
    };
    let mut plan = plan;
    let strip = value
        .get("strip_timing_state")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if strip {
        strip_volatile(&mut plan);
    }
    serde_json::json!({"ok": true, "fingerprint": overlay_fingerprint::fingerprint_plan(&plan)})
        .to_string()
}

pub fn run_stdio() -> io::Result<()> {
    ndjson_stdio::run_stdio(process_line)
}

#[cfg(test)]
mod tests {
    use super::process_line;
    use serde_json::Value;

    fn analyze_request() -> String {
        serde_json::json!({
            "operation": "analyze",
            "scenes": [
                {"scene_id": "scene-02", "text": "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini.", "language": "it", "video_start_ms": 20000, "video_end_ms": 40000},
                {"scene_id": "scene-01", "text": "Nel 2020 i costi aumentarono.", "language": "it", "video_start_ms": 0, "video_end_ms": 20000}
            ],
            "entities_per_scene": [["Ford", "Toyota"], []]
        })
        .to_string()
    }

    #[test]
    fn batch_analyze_returns_sorted_manifest_in_one_call() {
        // P0.1: two scenes, ONE request, ONE manifest with both scenes.
        let response: Value = serde_json::from_str(&process_line(&analyze_request())).unwrap();
        assert_eq!(response["ok"], true);
        let scenes = response["manifest"]["scenes"].as_array().unwrap();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0]["scene_id"], "scene-01");
        assert_eq!(scenes[1]["scene_id"], "scene-02");
        assert_eq!(
            response["manifest"]["schema_version"],
            "scene_intelligence.v1"
        );
    }

    #[test]
    fn evidence_indices_are_scene_local_in_worker_manifest() {
        let request = serde_json::json!({
            "operation": "analyze",
            "scenes": [
                {"scene_id": "scene-02", "text": "Ford collabora con Toyota. Ford cresce mentre Toyota rallenta.", "language": "it", "video_start_ms": 1000, "video_end_ms": 3000},
                {"scene_id": "scene-01", "text": "Nel 2020 i costi aumentarono.", "language": "it", "video_start_ms": 0, "video_end_ms": 1000}
            ],
            "entities_per_scene": [["Ford", "Toyota"], []]
        }).to_string();
        let response: Value = serde_json::from_str(&process_line(&request)).unwrap();
        assert_eq!(response["ok"], true, "{}", response);
        let later_scene = response["manifest"]["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|scene| scene["scene_id"] == "scene-02")
            .unwrap();
        let relations = later_scene["relations"].as_array().unwrap();
        assert!(
            !relations.is_empty(),
            "fixture must produce an entity relation"
        );
        assert_eq!(
            relations[0]["evidence_sentence_ids"][0], 0,
            "global sentence id 1 must be localized to scene sentence 0"
        );
        let temporal_events = response["manifest"]["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|scene| scene["scene_id"] == "scene-01")
            .unwrap()["temporal_events"]
            .as_array()
            .unwrap();
        assert_eq!(
            temporal_events[0]["narration_index"], 0,
            "the first scene's year remains local sentence 0"
        );
    }

    #[test]
    fn batch_analyze_request_order_does_not_matter() {
        let swapped = serde_json::json!({
            "operation": "analyze",
            "scenes": [
                {"scene_id": "scene-01", "text": "Nel 2020 i costi aumentarono.", "language": "it", "video_start_ms": 0, "video_end_ms": 20000},
                {"scene_id": "scene-02", "text": "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini.", "language": "it", "video_start_ms": 20000, "video_end_ms": 40000}
            ],
            "entities_per_scene": [[], ["Ford", "Toyota"]]
        })
        .to_string();
        let a: Value = serde_json::from_str(&process_line(&analyze_request())).unwrap();
        let b: Value = serde_json::from_str(&process_line(&swapped)).unwrap();
        assert_eq!(a["manifest"], b["manifest"]);
    }

    #[test]
    fn batch_analyze_rejects_empty_and_mismatched_inputs() {
        let empty: Value =
            serde_json::from_str(&process_line(r#"{"operation":"analyze","scenes":[]}"#)).unwrap();
        assert_eq!(empty["ok"], false);
        let mismatch: Value = serde_json::from_str(&process_line(
            r#"{"operation":"analyze","scenes":[{"scene_id":"s","text":"Ciao mondo.","language":"it","video_start_ms":0,"video_end_ms":1000}],"entities_per_scene":[]}"#,
        ))
        .unwrap();
        assert_eq!(mismatch["ok"], false);
        let bad_window: Value = serde_json::from_str(&process_line(
            r#"{"operation":"analyze","scenes":[{"scene_id":"s","text":"Ciao mondo.","language":"it","video_start_ms":5,"video_end_ms":5}]}"#,
        ))
        .unwrap();
        assert_eq!(bad_window["ok"], false);
    }

    #[test]
    fn split_sentences_preserves_utf8_byte_offsets() {
        let transcript = "La dott. Rossi è arrivata. Poi ha parlato!";
        let request = serde_json::json!({
            "operation": "split_sentences",
            "text": transcript,
            "scene_id": "s-01",
            "language": "it",
        })
        .to_string();
        let response: Value = serde_json::from_str(&process_line(&request)).unwrap();
        let sentences = response["sentences"].as_array().unwrap();
        assert_eq!(sentences.len(), 2);
        for sentence in sentences {
            let start = sentence["start_byte"].as_u64().unwrap() as usize;
            let end = sentence["end_byte"].as_u64().unwrap() as usize;
            assert!(transcript.is_char_boundary(start));
            assert!(transcript.is_char_boundary(end));
            assert_eq!(&transcript[start..end], sentence["text"].as_str().unwrap());
        }
    }

    #[test]
    fn preflight_op_mirrors_go_gate() {
        let ok: Value = serde_json::from_str(&process_line(
            r#"{"operation":"preflight","certified_audio_ms":5000,"scenes":[{"scene_id":"s-01","duration_seconds":4.5,"media_locator":"drive:abc"}]}"#,
        ))
        .unwrap();
        assert_eq!(ok["ok"], true);
        let bad: Value = serde_json::from_str(&process_line(
            r#"{"operation":"preflight","certified_audio_ms":0,"scenes":[]}"#,
        ))
        .unwrap();
        assert_eq!(bad["ok"], false);
    }

    #[test]
    fn fingerprint_op_strips_volatile_keys_by_default() {
        let with_timing = serde_json::json!({
            "operation": "fingerprint",
            "plan": {"a": 1, "timing_state": "frozen", "nested": {"attempt_id": "x", "v": 2}}
        })
        .to_string();
        let stripped_only = serde_json::json!({
            "operation": "fingerprint",
            "plan": {"a": 1, "nested": {"v": 2}}
        })
        .to_string();
        let a: Value = serde_json::from_str(&process_line(&with_timing)).unwrap();
        let b: Value = serde_json::from_str(&process_line(&stripped_only)).unwrap();
        assert_eq!(a["ok"], true);
        assert_eq!(a["fingerprint"], b["fingerprint"]);
    }

    #[test]
    fn unknown_operation_fails_closed() {
        let response: Value =
            serde_json::from_str(&process_line(r#"{"operation":"teleport"}"#)).unwrap();
        assert_eq!(response["ok"], false);
    }
}
