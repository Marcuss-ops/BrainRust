//! Common newline-delimited JSON framing for persistent Rust workers.
//!
//! Workers keep ownership of parsing, operation semantics, response envelopes,
//! and request error handling. This crate handles only line framing: each
//! nonblank stdin line is passed to the worker and produces exactly one JSON
//! response line. Stdin/stdout I/O failures are returned to the caller.

use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::sync::Mutex;

/// Response used when a request handler panics. One panic must still produce
/// exactly one JSON line, so a persistent client never waits forever.
const PANIC_RESPONSE: &str = "{\"ok\":false,\"error\":\"worker panicked\"}";

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
        let response =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process_line(&line)))
                .unwrap_or_else(|_| PANIC_RESPONSE.to_string());
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

/// One request line and its position in the input stream. The position is
/// what makes the parallel loop deterministic: responses are written in input
/// order no matter which worker finished first.
struct Job {
    index: usize,
    line: String,
}

/// Hard ceiling for an explicitly configured worker pool. An operator can
/// lower the pool per deployment, but no environment value may create an
/// unbounded number of threads for one request stream.
pub const MAX_WORKERS: usize = 32;

/// Number of worker threads for a persistent worker process.
///
/// `var` (for example `VISUALNER_WORKERS`) overrides the pool size; values
/// that are absent, unparsable or zero fall back to the machine's available
/// parallelism, clamped to a small ceiling so one worker cannot saturate a
/// shared host by itself. An explicit value is clamped to [`MAX_WORKERS`].
/// `1` restores the serial loop exactly.
pub fn workers_from_env(var: &str) -> usize {
    if let Some(value) = std::env::var(var)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
    {
        return value.min(MAX_WORKERS);
    }
    default_workers()
}

/// Available parallelism clamped to `[1, 8]`: beyond that the workers are
/// contending for the same cores and the latency of a single request grows
/// without increasing throughput.
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, 8)
}

/// Runs a JSON-lines worker with a bounded thread pool on stdin/stdout.
///
/// Responses are byte-identical to [`run_stdio`]: one line per non-blank
/// request, in input order, each flushed as soon as it is ready. Requests are
/// processed concurrently, so a worker whose `process_line` is CPU-bound uses
/// the whole machine instead of one core.
pub fn run_stdio_parallel(
    process_line: impl Fn(&str) -> String + Sync,
    workers: usize,
) -> io::Result<()> {
    // Owned handles, not locks: the reader runs on its own thread so a
    // response is written as soon as it is ready, even while the client keeps
    // stdin open waiting for exactly that response.
    run_io_parallel(
        io::BufReader::new(io::stdin()),
        io::stdout(),
        process_line,
        workers,
    )
}

/// [`run_io`] with a worker pool. Ordering is preserved by indexing every
/// request and emitting completed responses only when their index is next.
///
/// Reading happens on a dedicated thread: a persistent client that sends one
/// request and keeps stdin open waiting for the answer must still receive it,
/// so the writer loop can never depend on the next input line arriving.
pub fn run_io_parallel<R, W>(
    input: R,
    output: W,
    process_line: impl Fn(&str) -> String + Sync,
    workers: usize,
) -> io::Result<()>
where
    R: BufRead + Send,
    W: Write,
{
    let workers = workers.max(1);
    let mut output = io::BufWriter::new(output);
    // Bounded queue: at most `workers * 4` requests are in flight, so a slow
    // consumer cannot make the worker accumulate unbounded input.
    let window = workers.saturating_mul(4).max(1);
    let (job_tx, job_rx) = mpsc::sync_channel::<Job>(window);
    let job_rx = Mutex::new(job_rx);
    let (result_tx, result_rx) = mpsc::sync_channel::<(usize, String)>(window);
    let (permit_tx, permit_rx) = mpsc::sync_channel::<()>(window);
    for _ in 0..window {
        permit_tx
            .send(())
            .map_err(|_| io::Error::other("ndjson permit channel closed"))?;
    }
    let permit_rx = Mutex::new(permit_rx);
    let mut permit_tx = Some(permit_tx);

    std::thread::scope(|scope| -> io::Result<()> {
        // Reader: assigns each request its input position and stops at EOF or
        // when the workers are gone.
        let reader = scope.spawn(move || -> io::Result<()> {
            let mut index = 0usize;
            for line in input.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                // Acquire one in-flight slot before issuing the job. Slots
                // are released only after the ordered response is written.
                match permit_rx.lock() {
                    Ok(guard) => {
                        if guard.recv().is_err() {
                            return Ok(());
                        }
                    }
                    Err(_) => return Ok(()),
                }
                if job_tx.send(Job { index, line }).is_err() {
                    return Ok(());
                }
                index += 1;
            }
            Ok(())
        });
        for _ in 0..workers {
            let receiver = &job_rx;
            let sender = result_tx.clone();
            let process = &process_line;
            scope.spawn(move || loop {
                let job = match receiver.lock() {
                    Ok(guard) => guard.recv(),
                    Err(_) => break,
                };
                let Ok(job) = job else { break };
                // A panic in one request must produce one error line, not
                // kill the persistent worker or leave the client waiting.
                let response =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(&job.line)))
                        .unwrap_or_else(|_| PANIC_RESPONSE.to_string());
                if sender.send((job.index, response)).is_err() {
                    break;
                }
            });
        }
        drop(result_tx);

        let mut next_written = 0usize;
        let mut pending: BTreeMap<usize, String> = BTreeMap::new();
        let mut write_error: Option<io::Error> = None;
        'responses: while let Ok((index, response)) = result_rx.recv() {
            pending.insert(index, response);
            while let Some(response) = pending.remove(&next_written) {
                if let Err(error) = writeln!(output, "{response}").and_then(|()| output.flush()) {
                    // Closing the permit sender unblocks the reader, which
                    // then drops the job sender and lets every worker exit.
                    // Without this, a failed stdout write would leave the
                    // reader waiting for a permit that can never be released.
                    write_error = Some(error);
                    permit_tx.take();
                    break 'responses;
                }
                // The slot is free only once its ordered response is on the
                // wire; this is what bounds `pending`.
                if let Some(permit_tx) = permit_tx.as_ref() {
                    let _ = permit_tx.send(());
                }
                next_written += 1;
            }
        }
        // Stop accepting results before joining the reader: workers blocked
        // on a full result channel must observe the closed receiver.
        drop(result_rx);
        drop(pending);
        let reader_result = match reader.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::other("ndjson input reader panicked")),
        };
        if let Some(error) = write_error {
            return Err(error);
        }
        reader_result?;
        output.flush()?;
        Ok(())
    })
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
    fn parallel_pool_writes_responses_in_input_order() {
        // Requests finish in reverse order (earlier lines sleep longer); the
        // output must still match the serial loop byte for byte.
        let input: Vec<u8> = (0..24)
            .map(|index| format!("{{\"n\":{index}}}\n"))
            .collect::<String>()
            .into_bytes();
        let process = |line: &str| {
            let n: u64 = line
                .trim_start_matches("{\"n\":")
                .trim_end_matches('}')
                .parse()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(24 - n));
            format!("{{\"call\":{n}}}")
        };
        let mut serial = Vec::new();
        run_io(Cursor::new(input.clone()), &mut serial, process).unwrap();
        let mut parallel = Vec::new();
        super::run_io_parallel(Cursor::new(input), &mut parallel, process, 4).unwrap();
        assert_eq!(parallel, serial);
        assert_eq!(parallel.iter().filter(|byte| **byte == b'\n').count(), 24);
    }

    #[cfg(unix)]
    #[test]
    fn parallel_pool_answers_before_stdin_reaches_eof() {
        use std::io::{BufReader, Write as _};
        use std::os::unix::net::UnixStream;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        struct SharedSink(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for SharedSink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        // A persistent client sends one request and keeps its end open while
        // it waits for the answer: the pool must not wait for the next line
        // (or EOF) before flushing the response.
        let (mut client, server) = UnixStream::pair().unwrap();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer = SharedSink(Arc::clone(&sink));
        let handle = std::thread::spawn(move || {
            super::run_io_parallel(
                BufReader::new(server),
                writer,
                |line| line.to_uppercase(),
                3,
            )
        });
        client.write_all(b"hello\n").unwrap();
        client.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !sink.lock().unwrap().starts_with(b"HELLO\n") {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            sink.lock().unwrap().as_slice(),
            b"HELLO\n",
            "the response must be written while stdin is still open"
        );
        drop(client);
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn explicitly_configured_worker_counts_are_clamped() {
        let variable = "NDJSON_STDIO_TEST_WORKER_CEILING";
        std::env::set_var(variable, "100000");
        assert_eq!(super::workers_from_env(variable), super::MAX_WORKERS);
        std::env::set_var(variable, "1");
        assert_eq!(super::workers_from_env(variable), 1);
        std::env::remove_var(variable);
    }

    #[test]
    fn parallel_pool_terminates_when_the_output_writer_fails() {
        use std::sync::mpsc;
        use std::time::Duration;

        struct FailingWriter;
        impl std::io::Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic output failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        // More than the in-flight window so the reader is still admitting
        // requests when the first response fails to write. A deadlock here
        // would leave the reader blocked forever on a permit.
        let input: Vec<u8> = (0..128)
            .map(|index| format!("{{\"n\":{index}}}\n"))
            .collect::<String>()
            .into_bytes();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut writer = FailingWriter;
            let result = super::run_io_parallel(
                std::io::Cursor::new(input),
                &mut writer,
                |line| line.to_string(),
                2,
            );
            let _ = done_tx.send(result.is_err());
        });
        let failed = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("a failed response write must terminate the pool, not deadlock it");
        assert!(failed, "the write error must be returned to the caller");
        handle.join().expect("pool thread");
    }

    #[test]
    fn parallel_pool_with_one_worker_is_the_serial_loop() {
        let input = Cursor::new(b"a\n\nb\nc\n".to_vec());
        let mut output = Vec::new();
        super::run_io_parallel(input, &mut output, |line| line.to_uppercase(), 1).unwrap();
        assert_eq!(output, b"A\nB\nC\n");
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

    #[test]
    fn parallel_pool_bounds_in_flight_work_behind_a_slow_head() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};

        let input: Vec<u8> = (0..100)
            .map(|index| format!("{{\"n\":{index}}}\n"))
            .collect::<String>()
            .into_bytes();
        let processed = Arc::new(AtomicUsize::new(0));
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Arc::new(std::sync::Mutex::new(gate_rx));
        let processed_in_process = Arc::clone(&processed);
        let process = move |line: &str| {
            let n: usize = line
                .trim_start_matches("{\"n\":")
                .trim_end_matches('}')
                .parse()
                .expect("test request index");
            if n == 0 {
                let _ = gate_rx.lock().expect("test gate").recv();
            }
            processed_in_process.fetch_add(1, Ordering::SeqCst);
            format!("{{\"call\":{n}}}")
        };
        let mut output = Vec::new();
        let handle = std::thread::spawn(move || {
            super::run_io_parallel(Cursor::new(input), &mut output, process, 2)
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut last = 0;
        let mut stable = 0;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
            let now = processed.load(Ordering::SeqCst);
            if now == last {
                stable += 1;
                if stable >= 3 {
                    break;
                }
            } else {
                stable = 0;
                last = now;
            }
        }
        let in_flight = processed.load(Ordering::SeqCst);
        assert!(
            in_flight <= 8,
            "pool buffered {in_flight} requests behind a slow head"
        );
        gate_tx.send(()).expect("release slow head");
        handle.join().expect("pool thread").expect("pool result");
        assert_eq!(processed.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn parallel_pool_turns_a_panicking_request_into_one_error_line() {
        let input = Cursor::new(b"{\"n\":0}\n{\"n\":1}\n".to_vec());
        let mut output = Vec::new();
        super::run_io_parallel(
            input,
            &mut output,
            |line| {
                if line.contains("\"n\":0") {
                    panic!("synthetic worker panic");
                }
                "{\"ok\":true}".to_string()
            },
            2,
        )
        .expect("pool result");
        let text = String::from_utf8(output).expect("utf8 output");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("worker panicked"));
        assert_eq!(lines[1], "{\"ok\":true}");
    }
}
