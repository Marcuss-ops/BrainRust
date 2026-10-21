//! Dedicated newline-delimited JSON interface for phrase-impact analysis.
//! It performs no media I/O and stays separate from mediaexec.v1.
use crate::phrase_impact::{self, Request};
use std::io;

pub fn process_line(line: &str) -> String {
    let response = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(value) if value["operation"] == "split_sentences" => {
            let transcript = value["transcript"].as_str().unwrap_or_default();
            let language = value["language"].as_str().unwrap_or("en");
            serde_json::json!({"ok": true, "sentences": phrase_impact::split_sentences(transcript, language).iter().map(|sentence| serde_json::json!({"text": sentence.text, "start_byte": sentence.start_byte, "end_byte": sentence.end_byte})).collect::<Vec<_>>()})
        }
        Ok(value) => {
            let scene_inputs: Option<Vec<phrase_impact::SceneInput>> = value
                .get("scene_inputs")
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok());
            let scenes: Vec<String> =
                serde_json::from_value(value.get("scenes").cloned().unwrap_or_default())
                    .unwrap_or_default();
            let topics: Vec<String> =
                serde_json::from_value(value.get("scene_topics").cloned().unwrap_or_default())
                    .unwrap_or_default();
            let chapter_options = value.get("chapter_options").cloned().and_then(|options| {
                serde_json::from_value::<phrase_impact::ChapterOptions>(options).ok()
            });
            match serde_json::from_value::<Request>(value) {
                Ok(request) => {
                    let result = match (scene_inputs, chapter_options) {
                        (Some(inputs), Some(options)) => {
                            phrase_impact::run_with_scene_inputs(request, &inputs, &options)
                        }
                        (Some(inputs), None) => phrase_impact::run_with_scene_inputs(
                            request,
                            &inputs,
                            &phrase_impact::ChapterOptions::default(),
                        ),
                        (None, Some(options)) => phrase_impact::run_with_chapter_options(
                            request, &scenes, &topics, &options,
                        ),
                        (None, None) => {
                            phrase_impact::run_with_scene_context(request, &scenes, &topics)
                        }
                    };
                    match result {
                        Ok(result) => serde_json::json!({"ok": true, "result": result}),
                        Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}),
                    }
                }
                Err(error) => {
                    serde_json::json!({"ok": false, "error": format!("invalid phrase-impact request: {error}")})
                }
            }
        }
        Err(error) => {
            serde_json::json!({"ok": false, "error": format!("invalid phrase-impact request JSON: {error}")})
        }
    };
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!("{{\"ok\":false,\"error\":\"encode phrase-impact response: {error}\"}}")
    })
}

pub fn run_stdio() -> io::Result<()> {
    ndjson_stdio::run_stdio(process_line)
}

#[cfg(test)]
mod tests {
    use super::process_line;
    use serde_json::Value;

    #[test]
    fn returns_summary_bullets_and_heavy_sentences_without_media_protocol() {
        let response: Value = serde_json::from_str(&process_line(
            r#"{"transcript":"La squadra vinse il campionato. Il risultato cambiò la storia del club.","language":"it","embeddings":[],"lexical_only":true}"#,
        )).unwrap();
        assert_eq!(response["ok"], true);
        assert!(!response["result"]["summary"].as_str().unwrap().is_empty());
        assert!(!response["result"]["bullet_points"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!response["result"]["heavy_sentences"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn extracts_only_literal_numeric_chart_values_per_scene() {
        let response: Value = serde_json::from_str(&process_line(
            r#"{"transcript":"Output reached 42% before falling to €18.5; analysts reported thirteen jobs.","language":"en","embeddings":[],"lexical_only":true,"scene_inputs":[{"scene_id":"s1","text":"Output reached 42% before falling to €18.5; analysts reported thirteen jobs."}],"chapter_options":{"profile_version":"segmentation.v1","min_sentences":1,"max_sentences":36,"min_words":1,"context_sentences":1,"semantic_weight":1.0,"lexical_weight":1.0,"scene_weight":0.2,"evidence_weight":1.0,"complexity_penalty":1.0,"bullet_count":3}}"#,
        )).unwrap();
        assert_eq!(response["ok"], true);
        let scene = &response["result"]["scene_highlights"][0];
        let values = scene["chart_values"].as_array().unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], "42%");
        assert_eq!(values[1], "€18.5");
    }

    #[test]
    fn chart_value_literals_use_the_shared_locale_aware_numeric_parser() {
        assert_eq!(
            crate::phrase_impact::extract_chart_values(
                "Il costo era 12,5% e poi 1.200 euro.",
                "it"
            ),
            vec!["12,5%", "1.200"]
        );
        assert_eq!(
            crate::phrase_impact::extract_chart_values("The value was 1,200 and then 12.5%.", "en"),
            vec!["1,200", "12.5%"]
        );
    }

    #[test]
    fn splits_sentences_using_the_canonical_rust_boundary() {
        let transcript = "Il Milan vinse. Poi celebrò!";
        let response: Value = serde_json::from_str(&process_line(
            r#"{"operation":"split_sentences","transcript":"Il Milan vinse. Poi celebrò!","language":"it"}"#,
        )).unwrap();
        let sentences = response["sentences"].as_array().unwrap();
        assert_eq!(sentences.len(), 2);
        for sentence in sentences {
            let start = sentence["start_byte"].as_u64().unwrap() as usize;
            let end = sentence["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(
                &transcript.as_bytes()[start..end],
                sentence["text"].as_str().unwrap().as_bytes()
            );
        }
    }

    #[test]
    fn exposes_utf8_byte_offsets_for_the_full_synthetic_transcript() {
        let transcript =
            include_str!("../../semantic-core/fixtures/elon_musk_synthetic_transcript_it.txt");
        let request = serde_json::json!({
            "operation": "split_sentences",
            "transcript": transcript,
            "language": "it"
        });
        let response: Value = serde_json::from_str(&process_line(&request.to_string())).unwrap();
        let sentences = response["sentences"].as_array().unwrap();
        assert!(sentences.len() > 50);
        for sentence in sentences {
            let start = sentence["start_byte"].as_u64().unwrap() as usize;
            let end = sentence["end_byte"].as_u64().unwrap() as usize;
            assert!(transcript.is_char_boundary(start));
            assert!(transcript.is_char_boundary(end));
            assert_eq!(&transcript[start..end], sentence["text"].as_str().unwrap());
        }
    }

    #[test]
    fn reports_missing_vectors_when_lexical_mode_is_not_requested() {
        let response: Value = serde_json::from_str(&process_line(
            r#"{"transcript":"A complete sentence.","language":"en","embeddings":[]}"#,
        ))
        .unwrap();
        assert_eq!(response["ok"], false);
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("embedding count 0 does not match sentence count 1"));
    }
}
