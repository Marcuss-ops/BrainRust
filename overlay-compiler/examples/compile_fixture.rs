//! Compiles one `editorial_plan.v2` request fixture and prints the sealed plan.
//!
//! This is the fixture generator for TICKET-14 WP0: the checked-in plan
//! goldens under `tests/fixtures/overlay_plan/plans/` are produced by the real
//! planner, never hand-written. Regenerate with:
//!
//! ```sh
//! cargo run -p overlay-compiler --example compile_fixture -- \
//!   overlay-compiler/tests/fixtures/overlay_plan/requests/<name>.request.json \
//!   > overlay-compiler/tests/fixtures/overlay_plan/plans/<name>.plan.json
//! ```
//!
//! Once the Go cutover lands, these files are immutable contract inputs.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: compile_fixture <request.json>");
        return ExitCode::FAILURE;
    };
    let bytes = match std::fs::read_to_string(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let request: overlay_compiler::EditorialPlanV2Request = match serde_json::from_str(&bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("parse {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let compiled = match overlay_compiler::compile_overlay_plan_v2(&request) {
        Ok(compiled) => compiled,
        Err(error) => {
            eprintln!("compile {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match serde_json::to_string_pretty(&compiled) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("encode {path}: {error}");
            ExitCode::FAILURE
        }
    }
}
