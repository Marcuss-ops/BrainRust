//! Compatibility wrapper around the workspace NDJSON framing crate.

use std::io;

/// Runs the shared stdin/stdout line framing for PipelineGen workers.
pub fn run_stdio(process_line: impl FnMut(&str) -> String) -> io::Result<()> {
    ndjson_stdio::run_stdio(process_line)
}
