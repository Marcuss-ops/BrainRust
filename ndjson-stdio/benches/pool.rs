//! NDJSON worker-pool benchmark (harness = false, like the other benches in
//! this workspace).
//!
//! Measures the framing loop over a synthetic CPU-bound request: serial
//! (`run_io`) vs the bounded thread pool (`run_io_parallel`). The pool must
//! emit byte-identical output in input order; the bench asserts that before
//! reporting, so a throughput number is never printed for a wrong result.
//!
//! Run:
//! ```sh
//! cargo bench -p ndjson-stdio --bench pool
//! ```

use ndjson_stdio::{default_workers, run_io, run_io_parallel};
use std::io::Cursor;
use std::time::Instant;

const REQUESTS: usize = 120;
const MIX_ROUNDS: u64 = 40_000;

/// Deterministic CPU-bound stand-in for one worker request.
fn process(line: &str) -> String {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for _ in 0..MIX_ROUNDS {
        for byte in line.bytes() {
            acc ^= byte as u64;
            acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{{\"hash\":{acc}}}")
}

fn input() -> Vec<u8> {
    let mut payload = String::new();
    for index in 0..REQUESTS {
        payload.push_str(&format!("{{\"n\":{index},\"text\":\"scene {index}\"}}\n"));
    }
    payload.into_bytes()
}

fn measure(workers: usize) -> (f64, Vec<u8>) {
    let input = input();
    let started = Instant::now();
    let mut output = Vec::new();
    if workers == 1 {
        run_io(Cursor::new(input), &mut output, process).expect("serial run");
    } else {
        run_io_parallel(Cursor::new(input), &mut output, process, workers).expect("parallel run");
    }
    (started.elapsed().as_secs_f64() * 1000.0, output)
}

fn main() {
    let workers = default_workers();
    // One untimed warm-up of each path so the comparison sees steady state.
    let _ = measure(1);
    if workers > 1 {
        let _ = measure(workers);
    }

    let (serial_ms, serial_output) = measure(1);
    let serial_per_request = serial_ms / REQUESTS as f64;

    println!("── ndjson pool bench ──────────────────────────────");
    println!(
        "requests={REQUESTS} mix_rounds={MIX_ROUNDS} workers={workers} serial_ms={serial_ms:.1} serial_ms_per_request={serial_per_request:.3}"
    );
    if workers == 1 {
        println!("single-core host: the pool cannot be measured against serial");
        return;
    }
    let (parallel_ms, parallel_output) = measure(workers);
    assert_eq!(
        parallel_output, serial_output,
        "parallel output must be byte-identical to serial"
    );
    let speedup = serial_ms / parallel_ms;
    let parallel_per_request = parallel_ms / REQUESTS as f64;
    println!(
        "parallel_ms={parallel_ms:.1} parallel_ms_per_request={parallel_per_request:.3} speedup={speedup:.2}x throughput_serial_rps={:.1} throughput_parallel_rps={:.1}",
        REQUESTS as f64 * 1000.0 / serial_ms,
        REQUESTS as f64 * 1000.0 / parallel_ms
    );
}
