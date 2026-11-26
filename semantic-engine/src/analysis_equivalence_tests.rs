//! BR-02 gate: parallel phase 1 must not change a single byte of the manifest.
//!
//! The reference run uses one Rayon worker, the candidate run uses four. The
//! manifest is serialized with its canonical serde shape, so a difference in
//! identity ids, registration order, candidate ranking, event order or any
//! grounded span fails the comparison.

use super::*;

fn equivalence_request() -> ScriptRequest {
    ScriptRequest {
        language: "it".to_string(),
        reference_date: Some("2026-11-26".to_string()),
        entity_safety_cap: 0,
        editorial: None,
        scenes: vec![
            SceneInput {
                scene_id: "s01".to_string(),
                text: "Alberto Stasi ha fondato lo studio nel 2008 a Milano. Il progetto è costato 120 milioni di euro e ha coinvolto 42 tecnici.".to_string(),
            },
            SceneInput {
                scene_id: "s02".to_string(),
                text: "Il 15 marzo 2008 Stasi ha presentato il piano a Torino. La procura di Milano ha aperto un'indagine sul contratto.".to_string(),
            },
            SceneInput {
                scene_id: "s03".to_string(),
                text: "Andrea Sempio ha visitato il cantiere di Genova il 3 aprile 2009. Sempio ha incontrato due ingegneri e ha firmato il verbale.".to_string(),
            },
            SceneInput {
                scene_id: "s04".to_string(),
                text: "Nel 2010 il tribunale di Roma ha discusso il caso. La società ha perso il 18% del valore e 7 dipendenti hanno lasciato l'ufficio.".to_string(),
            },
            SceneInput {
                scene_id: "s05".to_string(),
                text: "Alberto Stasi è tornato a Milano per il processo. Il giudice ha ascoltato Stasi e Sempio nella stessa udienza.".to_string(),
            },
            SceneInput {
                scene_id: "s06".to_string(),
                text: "La procura ha chiesto una perizia sul contratto del 2008. Il perito ha stimato un danno di 3 milioni di euro.".to_string(),
            },
        ],
    }
}

fn manifest_bytes(threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("thread pool");
    let manifest = pool
        .install(|| analyze_script_with_scene_cache(&equivalence_request(), None))
        .expect("analysis succeeds");
    serde_json::to_vec(&manifest).expect("manifest encodes")
}

#[test]
fn one_thread_and_four_threads_produce_identical_bytes() {
    let serial = manifest_bytes(1);
    let parallel = manifest_bytes(4);
    assert_eq!(
        serial, parallel,
        "parallel scene analysis changed the manifest byte-for-byte"
    );
}

#[test]
fn repeated_parallel_runs_are_stable() {
    let first = manifest_bytes(4);
    let second = manifest_bytes(4);
    assert_eq!(first, second, "parallel analysis is not repeatable");
}

#[test]
fn scene_cache_round_trip_is_byte_identical() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let root = std::env::temp_dir().join(format!(
        "brainrust-scene-integration-{}-{nanos}",
        std::process::id()
    ));
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    let cold = analyze_script_with_scene_cache(&equivalence_request(), Some(&cache))
        .expect("cold analysis");
    let warm = analyze_script_with_scene_cache(&equivalence_request(), Some(&cache))
        .expect("warm analysis");
    assert_eq!(
        serde_json::to_vec(&cold).expect("cold encodes"),
        serde_json::to_vec(&warm).expect("warm encodes"),
        "a scene-cache hit changed the manifest"
    );
    let entries = std::fs::read_dir(&root)
        .expect("scene cache dir")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("scene-"))
        .count();
    assert_eq!(entries, equivalence_request().scenes.len());
    std::fs::remove_dir_all(root).expect("cleanup");
}
