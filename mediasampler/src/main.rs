use mediasampler::{sample_scene, Candidate, BoundAssets, SampleOptions, Scene, SampleResult};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

#[derive(Debug, Deserialize)]
struct SampleRequest {
    scene: Scene,
    candidates: Vec<Candidate>,
    #[serde(default)]
    allow_reuse: bool,
}

#[derive(Debug, Serialize)]
struct SampleResponse {
    results: Vec<SampleResult>,
    winner_id: Option<String>,
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("mediasampler read: {error}");
                std::process::exit(1);
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: SampleRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("mediasampler decode: {error}");
                std::process::exit(1);
            }
        };
        let mut bound = BoundAssets::new();
        let options = SampleOptions {
            allow_reuse: request.allow_reuse,
            images_per_scene: 0,
        };
        let (results, winner_id) = sample_scene(
            &request.scene,
            &request.candidates,
            &options,
            &mut bound,
        );
        serde_json::to_writer(&mut stdout, &SampleResponse { results, winner_id })
            .expect("mediasampler encode");
        stdout.write_all(b"\n").expect("mediasampler write");
        stdout.flush().expect("mediasampler flush");
    }
}
