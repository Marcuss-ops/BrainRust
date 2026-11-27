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

/// BR-AUDIT-08/18: the v4 projection must move the identity registry to one
/// document table without changing a single v3 byte or any non-identity field.
#[test]
fn v4_normalization_keeps_v3_byte_identical_and_collapses_identity_copies() {
    use std::collections::BTreeSet;

    let manifest =
        analyze_script_with_scene_cache(&equivalence_request(), None).expect("analysis succeeds");
    let v3_bytes = serde_json::to_vec(&manifest).expect("v3 manifest encodes");
    let v4_value = semantic_core::to_v4_value(&manifest).expect("v4 projection");
    assert_eq!(
        v3_bytes,
        serde_json::to_vec(&manifest).expect("v3 re-encodes"),
        "the v4 projection mutated the v3 manifest"
    );
    assert_eq!(
        v4_value,
        semantic_core::to_v4_value(&manifest).expect("v4 projection repeats"),
        "the v4 projection is not deterministic"
    );

    let v3: serde_json::Value = serde_json::from_slice(&v3_bytes).expect("v3 json");
    let v4: serde_json::Value = serde_json::from_value(v4_value).expect("v4 json");
    assert_eq!(v3["schema_version"], "semantic_manifest.v3");
    assert_eq!(v4["schema_version"], "semantic_manifest.v4");
    assert_eq!(v4["fingerprint"], v3["fingerprint"]);

    let global = v4["identities"].as_array().expect("global identity table");
    let mut ids = BTreeSet::new();
    for identity in global {
        let id = identity["id"].as_str().expect("identity id");
        assert!(ids.insert(id.to_string()), "duplicate identity {id}");
    }

    let mut copies = 0usize;
    let mut referenced = BTreeSet::new();
    for (index, scene) in v3["scenes"]
        .as_array()
        .expect("v3 scenes")
        .iter()
        .enumerate()
    {
        copies += scene["identities"]
            .as_array()
            .expect("v3 scene identities")
            .len();
        let scene_v4 = &v4["scenes"][index];
        assert!(
            scene_v4.get("identities").is_none(),
            "v4 scene {} still carries a cumulative identity copy",
            scene["scene_id"]
        );
        for id in scene_v4["identity_ids"]
            .as_array()
            .expect("v4 scene identity_ids")
        {
            let id = id.as_str().expect("identity id string");
            assert!(
                ids.contains(id),
                "scene {} references missing identity {id}",
                scene["scene_id"]
            );
            referenced.insert(id.to_string());
        }

        let mut v3_scene = scene.clone();
        v3_scene
            .as_object_mut()
            .expect("v3 scene object")
            .remove("identities");
        let mut v4_scene = scene_v4.clone();
        v4_scene
            .as_object_mut()
            .expect("v4 scene object")
            .remove("identity_ids");
        assert_eq!(
            v3_scene, v4_scene,
            "scene {} changed outside the identity table",
            scene["scene_id"]
        );
    }
    assert_eq!(
        referenced.len(),
        ids.len(),
        "every global identity must be referenced by at least one scene"
    );
    assert!(
        copies > global.len(),
        "corpus must exercise the cumulative copies: copies={copies} unique={}",
        global.len()
    );

    // The last occurrence of an id carries every mention; taking the first
    // would silently drop mentions added in later scenes.
    let repeated = global
        .iter()
        .find(|identity| {
            identity["mentions"]
                .as_array()
                .is_some_and(|mentions| mentions.len() > 1)
        })
        .expect("the corpus repeats at least one identity");
    let mention_scenes: BTreeSet<&str> = repeated["mentions"]
        .as_array()
        .expect("mentions")
        .iter()
        .filter_map(|mention| mention["scene_id"].as_str())
        .collect();
    assert!(
        mention_scenes.len() > 1,
        "repeated identity lost its later mentions"
    );

    let v4_bytes = serde_json::to_vec(&v4).expect("v4 manifest encodes");
    assert!(
        v4_bytes.len() < v3_bytes.len(),
        "v4 must be smaller on a corpus with repeated identities: v3={} v4={}",
        v3_bytes.len(),
        v4_bytes.len()
    );
    println!(
        "V4_WIRE_PROBE scenes={} v3_bytes={} v4_bytes={} identity_copies={} unique_identities={}",
        v3["scenes"].as_array().map_or(0, Vec::len),
        v3_bytes.len(),
        v4_bytes.len(),
        copies,
        global.len()
    );
}

#[test]
fn repeated_parallel_runs_are_stable() {
    let first = manifest_bytes(4);
    let second = manifest_bytes(4);
    assert_eq!(first, second, "parallel analysis is not repeatable");
}

/// Drops the wall-clock diagnostic block from the comparison. `editorial.timings`
/// records measured milliseconds; it is not semantic output and cannot be
/// byte-identical between two runs by construction. Every other field — ids,
/// registration order, candidate ranking, events, spans — is compared raw.
fn canonical_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return bytes.to_vec(),
    };
    if let Some(timings) = value
        .get_mut("editorial")
        .and_then(|e| e.get_mut("timings"))
    {
        *timings = serde_json::json!(null);
    }
    serde_json::to_vec(&value).expect("canonical manifest encodes")
}

/// Runs one request on an explicit Rayon pool and an explicit phase-1 window.
fn manifest_bytes_with(request: &ScriptRequest, threads: usize, window: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("thread pool");
    let manifest = pool
        .install(|| analyze_script_with_window(request, None, window))
        .expect("analysis succeeds");
    canonical_bytes(&serde_json::to_vec(&manifest).expect("manifest encodes"))
}

fn corpus_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/multilingual_100")
}

/// `(language, absolute path)` for every script in the 100-case corpus.
fn corpus_cases() -> Vec<(String, String)> {
    let root = corpus_root();
    let raw = std::fs::read_to_string(root.join("cases.json")).expect("cases.json");
    let cases: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("cases json");
    cases
        .iter()
        .filter_map(|case| {
            Some((
                case.get("lang")?.as_str()?.to_string(),
                root.join(case.get("path")?.as_str()?)
                    .to_string_lossy()
                    .into_owned(),
            ))
        })
        .collect()
}

/// Mirrors `fixtures/multilingual_100/benchmark.py`: blank-line blocks are
/// packed into scenes of at most `scene_words` words.
fn scenes_from_text(text: &str, scene_words: usize) -> Vec<SceneInput> {
    let mut scenes: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut words = 0;
    for block in text.trim().split("\n\n") {
        let block_words = block.split_whitespace().count();
        if words > 0 && words + block_words > scene_words {
            scenes.push(current.trim().to_string());
            current.clear();
            words = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(block);
        words += block_words;
    }
    if !current.trim().is_empty() {
        scenes.push(current.trim().to_string());
    }
    scenes
        .into_iter()
        .enumerate()
        .map(|(index, text)| SceneInput {
            scene_id: format!("s{:02}", index + 1),
            text,
        })
        .collect()
}

fn corpus_request(language: &str, path: &str) -> ScriptRequest {
    let text = std::fs::read_to_string(path).expect("corpus script");
    ScriptRequest {
        language: language.to_string(),
        scenes: scenes_from_text(&text, 140),
        reference_date: Some("2026-11-26".to_string()),
        editorial: None,
        entity_safety_cap: 0,
    }
}

/// First differing JSON line, so a parity failure names the field instead of
/// dumping two manifests into the test log.
fn first_json_difference(left: &[u8], right: &[u8]) -> String {
    let left: serde_json::Value = serde_json::from_slice(left).unwrap_or_default();
    let right: serde_json::Value = serde_json::from_slice(right).unwrap_or_default();
    let left = serde_json::to_string_pretty(&left).unwrap_or_default();
    let right = serde_json::to_string_pretty(&right).unwrap_or_default();
    for (index, (left, right)) in left.lines().zip(right.lines()).enumerate() {
        if left != right {
            return format!("line {}: left={left} right={right}", index + 1);
        }
    }
    "manifests differ in length only".to_string()
}

fn assert_thread_parity(request: &ScriptRequest, label: &str) {
    let reference = manifest_bytes_with(request, 1, usize::MAX);
    for threads in [2, 4, 8] {
        let candidate = manifest_bytes_with(request, threads, usize::MAX);
        assert_eq!(
            reference,
            candidate,
            "{label}: {threads} threads changed the manifest: {}",
            first_json_difference(&reference, &candidate)
        );
    }
}

/// Fast gate: one script per corpus language across four thread counts.
#[test]
fn multilingual_corpus_slice_is_byte_identical_across_thread_counts() {
    let mut seen = std::collections::BTreeSet::new();
    for (language, path) in corpus_cases() {
        if !seen.insert(language.clone()) {
            continue;
        }
        assert_thread_parity(&corpus_request(&language, &path), &path);
    }
    assert_eq!(seen.len(), 10, "the corpus must cover all ten languages");
}

/// Full certification corpus: all 100 scripts. Heavy; run explicitly.
#[test]
#[ignore = "certification corpus; run with --ignored --release"]
fn full_multilingual_corpus_is_byte_identical_across_thread_counts() {
    for (language, path) in corpus_cases() {
        assert_thread_parity(&corpus_request(&language, &path), &path);
    }
}

/// The twenty long gold scripts, one scene per paragraph.
#[test]
#[ignore = "certification corpus; run with --ignored --release"]
fn long_gold_scripts_are_byte_identical_across_thread_counts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/gold_multilingual/long_scripts");
    let mut scripts: Vec<_> = std::fs::read_dir(&root)
        .expect("long script dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    scripts.sort();
    assert_eq!(scripts.len(), 20, "the long gold corpus is release-sized");
    for path in scripts {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let language = name.split('_').next().expect("language prefix");
        let text = std::fs::read_to_string(&path).expect("long script");
        let request = ScriptRequest {
            language: language.to_string(),
            scenes: scenes_from_text(&text, 140),
            reference_date: Some("2026-11-26".to_string()),
            editorial: None,
            entity_safety_cap: 0,
        };
        assert_thread_parity(&request, &name);
    }
}

#[test]
fn editorial_input_is_byte_identical_across_thread_counts() {
    let text = "Alberto Stasi ha fondato lo studio nel 2008 a Milano. \
                Il progetto e' costato 120 milioni di euro e ha coinvolto 42 tecnici.";
    let editorial = serde_json::from_value(serde_json::json!({
        "request": {
            "transcript": text,
            "language": "it",
            "embeddings": [],
            "lexical_only": true
        }
    }))
    .expect("editorial request");
    let request = ScriptRequest {
        language: "it".to_string(),
        scenes: vec![SceneInput {
            scene_id: "s01".to_string(),
            text: text.to_string(),
        }],
        reference_date: Some("2026-11-26".to_string()),
        editorial: Some(editorial),
        entity_safety_cap: 0,
    };
    assert_thread_parity(&request, "editorial");
}

#[test]
fn scene_window_sizes_produce_identical_manifests() {
    let request = equivalence_request();
    let reference = manifest_bytes_with(&request, 4, usize::MAX);
    for window in [1, 2, 3, 64] {
        assert_eq!(
            reference,
            manifest_bytes_with(&request, 4, window),
            "window {window} changed the manifest"
        );
    }
}

fn temp_scene_root(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("brainrust-{label}-{}-{nanos}", std::process::id()))
}

fn scene_entries(root: &std::path::Path) -> usize {
    std::fs::read_dir(root)
        .expect("scene cache dir")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("scene-"))
        .count()
}

#[test]
fn scene_cache_misses_when_the_reference_date_changes() {
    let root = temp_scene_root("scene-date");
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    let mut request = equivalence_request();
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("first analysis");
    let first = scene_entries(&root);
    assert_eq!(first, request.scenes.len());

    request.reference_date = Some("2020-01-01".to_string());
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("second analysis");
    assert_eq!(
        scene_entries(&root),
        first + request.scenes.len(),
        "a different reference date must not reuse a scene draft"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scene_cache_misses_when_the_script_context_changes() {
    fn request(second_scene: &str) -> ScriptRequest {
        ScriptRequest {
            language: "it".to_string(),
            scenes: vec![
                SceneInput {
                    scene_id: "s01".to_string(),
                    text: "Mario Rossi ha parlato ieri a Milano.".to_string(),
                },
                SceneInput {
                    scene_id: "s02".to_string(),
                    text: second_scene.to_string(),
                },
            ],
            reference_date: Some("2026-11-26".to_string()),
            editorial: None,
            entity_safety_cap: 0,
        }
    }

    let root = temp_scene_root("scene-context");
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    analyze_script_with_window(
        &request("Paolo Bianchi ha firmato il contratto."),
        Some(&cache),
        usize::MAX,
    )
    .expect("first analysis");
    assert_eq!(scene_entries(&root), 2);

    analyze_script_with_window(
        &request("Luca Verdi ha firmato il contratto."),
        Some(&cache),
        usize::MAX,
    )
    .expect("second analysis");
    assert_eq!(
        scene_entries(&root),
        4,
        "the unchanged scene must miss when the global name context changes"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scene_cache_misses_when_the_entity_safety_cap_changes() {
    let root = temp_scene_root("scene-cap");
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    let mut request = equivalence_request();
    request.entity_safety_cap = 3;
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("first analysis");
    let first = scene_entries(&root);

    request.entity_safety_cap = 7;
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("second analysis");
    assert_eq!(
        scene_entries(&root),
        first + request.scenes.len(),
        "a different safety cap must not reuse a scene draft"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scene_cache_reuses_every_unmodified_scene() {
    let root = temp_scene_root("scene-partial");
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    let mut request = equivalence_request();
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("first analysis");
    let first = scene_entries(&root);

    request.scenes[1].text.push_str(" Un dettaglio nuovo.");
    analyze_script_with_window(&request, Some(&cache), usize::MAX).expect("second analysis");
    assert_eq!(
        scene_entries(&root),
        first + 1,
        "only the edited scene may be analysed again"
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scene_cache_rejects_an_older_format_and_a_corrupt_entry() {
    let root = temp_scene_root("scene-corrupt");
    let cache = cache::scene::SceneCache::open(root.clone(), 64 * 1024 * 1024).expect("cache");
    analyze_script_with_window(&equivalence_request(), Some(&cache), usize::MAX).expect("analysis");
    let key = std::fs::read_dir(&root)
        .expect("dir")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("scene-"))
        })
        .expect("one scene entry");
    let key = key
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("key")
        .to_string();

    let payload = std::fs::read(root.join(format!("{key}.json"))).expect("read entry");
    let mut value: serde_json::Value = serde_json::from_slice(&payload).expect("json");
    value["cache_format"] = serde_json::json!(1);
    std::fs::write(
        root.join(format!("{key}.json")),
        serde_json::to_vec(&value).expect("encode"),
    )
    .expect("write old format");
    assert!(
        cache.load(&key, "it").is_none(),
        "an older scene format must be a miss"
    );

    let path = root.join(format!("{key}.json"));
    std::fs::write(&path, b"{\"cache_format\":2,").expect("truncate");
    assert!(
        cache.load(&key, "it").is_none(),
        "a corrupt entry must miss"
    );
    assert!(!path.exists(), "a corrupt entry is deleted, not retried");
    std::fs::remove_dir_all(root).expect("cleanup");
}

/// BR-AUDIT-19 measurement: peak RSS for one long script. Run twice in
/// separate processes with `BRAINRUST_SCENE_WINDOW=1` and `=0` (unlimited):
///
/// ```sh
/// BRAINRUST_SCENE_WINDOW=1 cargo test -p semantic-engine --release \
///   long_script_peak_rss -- --ignored --nocapture
/// ```
#[test]
#[ignore = "manual RSS probe; run with --ignored --nocapture"]
fn long_script_peak_rss() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/gold_multilingual/long_scripts/it_stadium.txt");
    let text = std::fs::read_to_string(&path).expect("long script");
    let request = ScriptRequest {
        language: "it".to_string(),
        scenes: scenes_from_text(&text, 140),
        reference_date: Some("2026-11-26".to_string()),
        editorial: None,
        entity_safety_cap: 0,
    };
    let window = match std::env::var("BRAINRUST_SCENE_WINDOW")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
    {
        Some(0) | None => usize::MAX,
        Some(window) => window,
    };
    let manifest = analyze_script_with_window(&request, None, window).expect("analysis");
    let bytes = serde_json::to_vec(&manifest).expect("encode").len();
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let hwm = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap_or("VmHWM: unknown");
    println!(
        "RSS_PROBE window={window} scenes={} manifest_bytes={bytes} {hwm}",
        request.scenes.len()
    );
}

/// BR-AUDIT-08 measurement: wire/cache footprint of the same long analysis at
/// v3 and v4. The corpus repeats a real long script so identities accumulate
/// and the cumulative copies become visible. Run with:
///
/// ```sh
/// BRAINRUST_V4_REPEATS=10 cargo test -p semantic-engine --release \
///   manifest_v4_wire_probe -- --ignored --nocapture
/// ```
#[test]
#[ignore = "manual v4 wire probe; run with --ignored --nocapture"]
fn manifest_v4_wire_probe() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/gold_multilingual/long_scripts/it_stadium.txt");
    let text = std::fs::read_to_string(&path).expect("long script");
    let base = scenes_from_text(&text, 140);
    let repeats = std::env::var("BRAINRUST_V4_REPEATS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10);
    let scenes: Vec<SceneInput> = (0..repeats)
        .flat_map(|repeat| {
            base.iter().map(move |scene| SceneInput {
                scene_id: format!("r{repeat}_{}", scene.scene_id),
                text: scene.text.clone(),
            })
        })
        .collect();
    let request = ScriptRequest {
        language: "it".to_string(),
        scenes,
        reference_date: Some("2026-11-26".to_string()),
        editorial: None,
        entity_safety_cap: 0,
    };
    let manifest = analyze_script_with_window(&request, None, usize::MAX).expect("analysis");
    let v3_bytes = serde_json::to_vec(&manifest).expect("v3 encodes");
    let v4_value = semantic_core::to_v4_value(&manifest).expect("v4 projection");
    let v4_bytes = serde_json::to_vec(&v4_value).expect("v4 encodes");
    let copies: usize = manifest
        .scenes
        .iter()
        .map(|scene| scene.identities.len())
        .sum();
    let unique = semantic_core::global_identities(&manifest).len();
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let hwm = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap_or("VmHWM: unknown");
    let saved = v3_bytes.len().saturating_sub(v4_bytes.len());
    println!(
        "V4_WIRE_PROBE scenes={} repeats={repeats} v3_bytes={} v4_bytes={} saved_bytes={saved} saved_pct={:.2} identity_copies={copies} unique_identities={unique} {hwm}",
        manifest.scenes.len(),
        v3_bytes.len(),
        v4_bytes.len(),
        saved as f64 * 100.0 / v3_bytes.len().max(1) as f64,
    );
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
