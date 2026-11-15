//! Bounded-memory gate for TICKET-12 WP4.
//!
//! Run explicitly in release:
//!   cargo test -p semantic-engine --test resource_gate --release -- \
//!     --ignored --nocapture
//!
//! Every analysis uses different text, so the manifest cache fills to its
//! 256-entry / 128 MiB budget and then evicts. The last three samples must
//! stay within a small band; monotonic growth means the cache budget or the
//! analysis path is leaking.

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};

const ANALYSES: usize = 1_000;
const SAMPLE_EVERY: usize = 100;

fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("VmRSS:")?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

#[test]
#[ignore = "1000-analysis RSS gate; run explicitly in release"]
fn thousand_analyses_reach_an_rss_plateau() {
    let mut samples: Vec<(usize, u64)> = Vec::new();
    for index in 0..ANALYSES {
        let request = ScriptRequest {
            language: "it".to_string(),
            scenes: vec![SceneInput {
                scene_id: format!("scene-{index:04}"),
                text: format!(
                    "SpaceX e NASA hanno presentato la missione {index} a Roma. Michael Jordan ha parlato con ESA."
                ),
            }],
            editorial: None,
            entity_safety_cap: 0,
        };
        analyze_script(&request).expect("analysis");
        if (index + 1) % SAMPLE_EVERY == 0 {
            if let Some(kib) = rss_kib() {
                samples.push((index + 1, kib));
                eprintln!("RSS after {} analyses: {kib} KiB", index + 1);
            }
        }
    }

    assert!(
        samples.len() >= 5,
        "expected RSS samples every {SAMPLE_EVERY} analyses, got {samples:?}"
    );
    let tail = &samples[samples.len() - 3..];
    let min = tail.iter().map(|(_, kib)| *kib).min().unwrap_or(0);
    let max = tail.iter().map(|(_, kib)| *kib).max().unwrap_or(0);
    let growth_kib = max.saturating_sub(min);
    eprintln!(
        "RSS plateau over the last {} analyses: min={min} KiB max={max} KiB growth={growth_kib} KiB",
        SAMPLE_EVERY * 3
    );
    assert!(
        growth_kib < 64 * 1024,
        "RSS did not plateau: {growth_kib} KiB growth across the last three samples: {samples:?}"
    );
}
