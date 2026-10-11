//! Common newline-delimited JSON framing for persistent Rust workers.
//!
//! Workers keep ownership of parsing, operation semantics, response envelopes,
//! and request error handling. This crate handles only line framing: each
//! nonblank stdin line is passed to the worker and produces exactly one JSON
//! response line. Stdin/stdout I/O failures are returned to the caller.

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

#[cfg(test)]
mod tests {
    use super::run_io;
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
}
