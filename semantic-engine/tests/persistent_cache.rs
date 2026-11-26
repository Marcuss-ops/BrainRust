//! Persistent cache across real worker processes (TICKET-37).
//!
//! The in-process unit tests prove the store's rules. These tests prove the
//! operator-facing claims: a manifest written by one `semantic-engine` process
//! is served from disk by the next one, concurrent writers publish one atomic
//! entry, and a model-free worker never reuses a model-backed entry.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const TEXT: &str = "Alberto Stasi e Andrea Sempio sono due nomi.";

fn temp_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "semantic-engine-persistent-cache-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("temp cache dir");
    root
}

fn request() -> String {
    serde_json::json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": TEXT}]
    })
    .to_string()
}

/// Starts one worker, writes exactly one request and returns the child with its
/// stdin closed. `models: None` points `VISUALNER_MODEL_DIR` at an empty
/// directory, because removing the variable would fall back to
/// `$HOME/.local/share/visualner/models`, which may hold real models.
fn spawn_worker(cache_dir: &Path, models: Option<&Path>) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-engine"));
    command
        .env("SEMANTIC_ENGINE_WORKERS", "1")
        .env("SEMANTIC_CACHE_DIR", cache_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match models {
        Some(models) => {
            command.env("VISUALNER_MODEL_DIR", models);
        }
        None => {
            // A dedicated empty directory: removing the variable would fall
            // back to `$HOME/.local/share/visualner/models`, which may hold
            // real models, and the unusable-cache test cannot derive a sibling
            // path from its cache directory.
            let empty = std::env::temp_dir()
                .join(format!("semantic-engine-no-models-{}", std::process::id()));
            std::fs::create_dir_all(&empty).expect("empty model dir");
            command.env("VISUALNER_MODEL_DIR", empty);
        }
    }
    let mut child = command.spawn().expect("spawn semantic-engine");
    {
        let mut stdin = child.stdin.take().expect("worker stdin");
        stdin
            .write_all(request().as_bytes())
            .expect("write request");
        stdin.write_all(b"\n").expect("terminate request line");
    }
    child
}

fn finish(child: Child) -> serde_json::Value {
    let output = child.wait_with_output().expect("worker exits on EOF");
    assert!(
        output.status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 worker output");
    let responses: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(responses.len(), 1, "one request, one response: {stdout}");
    serde_json::from_str(responses[0]).expect("worker returns one JSON object")
}

fn analyze_in_new_process(cache_dir: &Path) -> serde_json::Value {
    finish(spawn_worker(cache_dir, None))
}

/// Manifest entries only. The same directory also holds the per-scene
/// phase-1 cache under `scenes/`; that store has its own tests and must not
/// change how many MANIFESTS a worker wrote.
fn entries(cache_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(cache_dir)
        .expect("cache dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("manifest-"))
        .collect();
    names.sort();
    names
}

#[test]
fn a_manifest_survives_a_worker_restart() {
    let cache_dir = temp_root();

    let cold = analyze_in_new_process(&cache_dir);
    assert_eq!(cold["ok"], true, "cold worker must analyse the script");
    assert_eq!(
        cold["cached"], false,
        "the first process cannot have a disk hit"
    );

    let written = entries(&cache_dir);
    assert_eq!(written.len(), 1, "one manifest entry, got {written:?}");
    assert!(
        written[0].starts_with("manifest-") && written[0].ends_with(".json"),
        "entry is content-addressed: {written:?}"
    );

    let warm = analyze_in_new_process(&cache_dir);
    assert_eq!(warm["ok"], true);
    assert_eq!(
        warm["cached"], true,
        "a second process must serve the manifest from disk"
    );
    assert_eq!(
        warm["manifest"], cold["manifest"],
        "a disk hit must return the same manifest as a fresh analysis"
    );
    assert_eq!(
        warm["completeness"], cold["completeness"],
        "a disk hit must report the same completeness contract"
    );

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn concurrent_workers_publish_one_atomic_entry() {
    let cache_dir = temp_root();

    let children: Vec<Child> = (0..8).map(|_| spawn_worker(&cache_dir, None)).collect();
    let responses: Vec<serde_json::Value> = children.into_iter().map(finish).collect();
    for response in &responses {
        assert_eq!(response["ok"], true);
    }
    let first = responses[0]["manifest"].clone();
    for response in &responses[1..] {
        assert_eq!(
            response["manifest"], first,
            "every concurrent writer must publish the same manifest"
        );
    }

    let written = entries(&cache_dir);
    assert_eq!(
        written.len(),
        1,
        "one entry after eight writers: {written:?}"
    );
    assert!(
        written.iter().all(|name| !name.ends_with(".part")),
        "no partial file may survive: {written:?}"
    );

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn a_model_free_and_a_model_backed_worker_keep_separate_entries() {
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp/udpipe-models");
    if !models.is_dir() {
        eprintln!(
            "skipping runtime-profile separation: no models at {}",
            models.display()
        );
        return;
    }
    let cache_dir = temp_root();

    let degraded = finish(spawn_worker(&cache_dir, None));
    let healthy = finish(spawn_worker(&cache_dir, Some(&models)));
    assert_eq!(degraded["ok"], true);
    assert_eq!(healthy["ok"], true);
    assert_eq!(degraded["cached"], false);
    assert_eq!(
        healthy["cached"], false,
        "a model-backed worker must not read the degraded entry"
    );
    assert_eq!(
        entries(&cache_dir).len(),
        2,
        "each runtime profile owns one entry"
    );

    let healthy_again = finish(spawn_worker(&cache_dir, Some(&models)));
    assert_eq!(
        healthy_again["cached"], true,
        "the model-backed entry stays warm"
    );
    assert_eq!(healthy_again["manifest"], healthy["manifest"]);

    let degraded_again = finish(spawn_worker(&cache_dir, None));
    assert_eq!(
        degraded_again["cached"], true,
        "the degraded entry stays separate and warm"
    );

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn an_unusable_cache_dir_never_fails_the_request() {
    let root = temp_root();
    // A regular file where the cache directory should be: `create_dir_all`
    // fails, so the worker must degrade to memory-only instead of refusing the
    // job (or panicking at startup).
    let blocked = root.join("not-a-directory");
    std::fs::write(&blocked, b"blocked").expect("blocking file");
    let unusable = blocked.join("cache");

    let response = finish(spawn_worker(&unusable, None));
    assert_eq!(response["ok"], true, "a broken cache must not fail the job");
    assert_eq!(response["cached"], false);

    assert_eq!(std::fs::read(&blocked).expect("blocking file"), b"blocked");
    let _ = std::fs::remove_dir_all(root);
}
