use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use visualner::{extract, supported_spellout_language_count, ExtractOptions, VisualEntity};

#[derive(Debug, Deserialize)]
struct ExtractRequest {
    source_text: String,
    language: String,
    #[serde(default)]
    entity_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_requires_language() {
        let missing =
            serde_json::from_str::<ExtractRequest>(r#"{"source_text":"twenty-five percent"}"#);
        assert!(
            missing.is_err(),
            "missing language must be rejected at the CLI boundary"
        );

        let present = serde_json::from_str::<ExtractRequest>(
            r#"{"source_text":"twenty-five percent","language":"en"}"#,
        );
        assert_eq!(present.unwrap().language, "en");
    }
}

#[derive(Debug, Serialize)]
struct ExtractResponse {
    entities: Vec<VisualEntity>,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--locale-count") {
        println!("{}", supported_spellout_language_count());
        return;
    }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("visualner read: {error}");
                std::process::exit(1);
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: ExtractRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("visualner decode: {error}");
                std::process::exit(1);
            }
        };
        let entities = match extract(
            &request.source_text,
            &ExtractOptions {
                language: request.language,
                entity_count: request.entity_count,
            },
        ) {
            Ok(entities) => entities,
            Err(error) => {
                eprintln!("visualner extract: {error}");
                std::process::exit(1);
            }
        };
        let response = ExtractResponse { entities };
        serde_json::to_writer(&mut stdout, &response).expect("visualner encode");
        stdout.write_all(b"\n").expect("visualner write");
        stdout.flush().expect("visualner flush");
    }
}
