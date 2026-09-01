use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use visualner::{extract, ExtractOptions, VisualEntity};

#[derive(Debug, Deserialize)]
struct ExtractRequest {
    source_text: String,
    #[serde(default)]
    entity_count: usize,
}

#[derive(Debug, Serialize)]
struct ExtractResponse {
    entities: Vec<VisualEntity>,
}

fn main() {
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
        let response = ExtractResponse {
            entities: extract(
                &request.source_text,
                &ExtractOptions { entity_count: request.entity_count },
            ),
        };
        serde_json::to_writer(&mut stdout, &response).expect("visualner encode");
        stdout.write_all(b"\n").expect("visualner write");
        stdout.flush().expect("visualner flush");
    }
}
