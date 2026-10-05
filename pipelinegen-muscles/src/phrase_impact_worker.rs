//! Dedicated newline-delimited JSON interface for phrase-impact analysis.
//! It performs no media I/O and stays separate from mediaexec.v1.
use crate::phrase_impact::{self, Request};
use std::io::{self, BufRead, Write};

pub fn process_line(line: &str) -> String {
    let response = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(value) if value["operation"] == "split_sentences" => {
            let transcript = value["transcript"].as_str().unwrap_or_default();
            let language = value["language"].as_str().unwrap_or("en");
            serde_json::json!({"ok": true, "sentences": phrase_impact::split_sentences(transcript, language).iter().map(|sentence| sentence.text.clone()).collect::<Vec<_>>()})
        }
        Ok(value) => match serde_json::from_value::<Request>(value) {
            Ok(request) => match phrase_impact::run(request) {
                Ok(result) => serde_json::json!({"ok": true, "result": result}),
                Err(error) => serde_json::json!({"ok": false, "error": error}),
            },
            Err(error) => {
                serde_json::json!({"ok": false, "error": format!("invalid phrase-impact request: {error}")})
            }
        },
        Err(error) => {
            serde_json::json!({"ok": false, "error": format!("invalid phrase-impact request JSON: {error}")})
        }
    };
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!("{{\"ok\":false,\"error\":\"encode phrase-impact response: {error}\"}}")
    })
}

pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        writeln!(stdout, "{}", process_line(&line))?;
        stdout.flush()?;
    }
    Ok(())
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
    fn splits_sentences_using_the_canonical_rust_boundary() {
        let response: Value = serde_json::from_str(&process_line(
            r#"{"operation":"split_sentences","transcript":"Il Milan vinse. Poi celebrò!","language":"it"}"#,
        )).unwrap();
        assert_eq!(response["sentences"].as_array().unwrap().len(), 2);
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
