//! Common newline-delimited JSON framing for persistent Rust workers.
//!
//! Workers keep ownership of parsing, operation semantics, response envelopes,
//! and request error handling. This crate handles only line framing: each
//! nonblank stdin line is passed to the worker and produces exactly one JSON
//! response line. Stdin/stdout I/O failures are returned to the caller.

use serde::Serialize;
use std::io::{self, BufRead, Write};

/// Runs a JSON-lines worker on stdin/stdout until EOF, skipping blank lines
/// and flushing every response for persistent request/response clients.
pub fn run_stdio(process_line: impl FnMut(&str) -> String) -> io::Result<()> {
    run_io(io::stdin().lock(), io::stdout().lock(), process_line)
}

/// Runs the framing loop over caller-provided streams.
pub fn run_io<R, W>(
    input: R,
    output: W,
    mut process_line: impl FnMut(&str) -> String,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    let mut output = io::BufWriter::new(output);
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        writeln!(output, "{}", process_line(&line))?;
        output.flush()?;
    }
    Ok(())
}

/// Machine-readable category of a worker rejection. Serialized as the
/// additive `error_code` field next to the human-readable `error` string, so
/// existing clients that only read `error` keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request line is not valid JSON for the worker's request type.
    Decode,
    /// The envelope `version` is not one the worker implements.
    UnsupportedVersion,
    /// The envelope `operation` is not one the worker implements.
    UnsupportedOperation,
    /// The request was well-formed but the operation itself failed.
    Execution,
    /// The worker could not encode its own response.
    Encode,
}

/// A rejected request. The worker keeps running and reports this on the
/// response line instead of exiting, because the caller multiplexes queued
/// requests onto the same persistent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerError {
    pub code: ErrorCode,
    pub message: String,
}

impl WorkerError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Writes the message to stderr for operators. Stdout stays reserved for
    /// response lines.
    pub fn log(&self) {
        eprintln!("{}", self.message);
    }
}

/// Fails closed on envelope values the worker does not implement. Absent
/// fields are accepted for forward compatibility with callers that omit them.
pub fn check_envelope(
    worker: &str,
    version: Option<&str>,
    expected_version: &str,
    operation: Option<&str>,
    expected_operation: &str,
) -> Result<(), WorkerError> {
    if let Some(version) = version {
        if version != expected_version {
            return Err(WorkerError::new(
                ErrorCode::UnsupportedVersion,
                format!("{worker}: unsupported version: {version}"),
            ));
        }
    }
    if let Some(operation) = operation {
        if operation != expected_operation {
            return Err(WorkerError::new(
                ErrorCode::UnsupportedOperation,
                format!("{worker}: unsupported operation: {operation}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_envelope, run_io, ErrorCode};
    use std::io::Cursor;

    #[test]
    fn skips_blank_lines_and_emits_exactly_one_response_per_request() {
        let input = Cursor::new(b"{\"n\":1}\n \n{\"n\":2}\n".to_vec());
        let mut calls = 0;
        let mut output = Vec::new();
        run_io(input, &mut output, |_| {
            calls += 1;
            format!("{{\"call\":{calls}}}")
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(output, b"{\"call\":1}\n{\"call\":2}\n");
    }

    #[test]
    fn envelope_accepts_absent_and_matching_values() {
        assert!(check_envelope("w", None, "w.v1", None, "run").is_ok());
        assert!(check_envelope("w", Some("w.v1"), "w.v1", Some("run"), "run").is_ok());
    }

    #[test]
    fn envelope_rejects_unknown_version_and_operation_with_codes() {
        let version = check_envelope("w", Some("w.v9"), "w.v1", None, "run").unwrap_err();
        assert_eq!(version.code, ErrorCode::UnsupportedVersion);
        assert_eq!(version.message, "w: unsupported version: w.v9");

        let operation = check_envelope("w", None, "w.v1", Some("drop"), "run").unwrap_err();
        assert_eq!(operation.code, ErrorCode::UnsupportedOperation);
        assert_eq!(operation.message, "w: unsupported operation: drop");
    }
}
