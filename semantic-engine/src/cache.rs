//! In-memory manifest cache for the persistent worker.
//!
//! A persistent worker sees the same script text again on replay and on
//! repeated jobs. Re-running the extractor is pure CPU for identical input, so
//! the manifest is memoized under the request **fingerprint** (FNV-1a over the
//! canonical language + scenes + analyzer versions). Equal text and equal
//! analyzer versions hit; any change to the text or to an analyzer version
//! yields a different fingerprint and misses, so the cache can never serve a
//! result computed by a different extractor build.

use std::collections::{HashMap, VecDeque};

use semantic_core::{AnalyzerVersions, SceneInput, ScriptRequest, SemanticManifest};

use crate::fingerprint;

/// Bounded FIFO cache from request fingerprint to analyzed manifest.
///
/// FIFO (not LRU) keeps the eviction deterministic and the memory bounded: a
/// script batch is analysed once and replayed, so recency ordering adds book
/// keeping without changing which results are correct. The bound only stops a
/// long-lived worker from growing without limit; it never changes output.
pub struct ManifestCache {
    max_entries: usize,
    entries: HashMap<String, SemanticManifest>,
    order: VecDeque<String>,
    hits: u64,
    misses: u64,
}

impl ManifestCache {
    /// A cache holding at most `max_entries` manifests. Zero disables caching.
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: HashMap::new(),
            order: VecDeque::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// The cache key for a request: the manifest fingerprint over the
    /// canonical language, scenes and analyzer versions.
    pub fn key(language: &str, scenes: &[SceneInput], analyzers: &AnalyzerVersions) -> String {
        fingerprint(language, scenes, analyzers)
    }

    /// Request-complete key used by the worker, including editorial options,
    /// scene context and caller-supplied embedding vectors.
    pub fn request_key(request: &ScriptRequest, analyzers: &AnalyzerVersions) -> String {
        crate::request_fingerprint(request, analyzers)
    }

    /// Returns the cached manifest for `key`, if present.
    pub fn get(&mut self, key: &str) -> Option<SemanticManifest> {
        if self.max_entries == 0 {
            return None;
        }
        match self.entries.get(key) {
            Some(manifest) => {
                self.hits += 1;
                Some(manifest.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Stores a manifest under `key`, evicting the oldest entries past the cap.
    pub fn insert(&mut self, key: String, manifest: SemanticManifest) {
        if self.max_entries == 0 {
            return;
        }
        if self.entries.insert(key.clone(), manifest).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cache counters for observability.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_core::SceneInput;

    fn manifest_with(language: &str, scenes: &[SceneInput]) -> SemanticManifest {
        crate::analyze_script(&semantic_core::ScriptRequest {
            language: language.to_string(),
            scenes: scenes.to_vec(),
            editorial: None,
            entity_safety_cap: 0,
        })
        .expect("analysis succeeds")
    }

    fn scene(id: &str, text: &str) -> SceneInput {
        SceneInput {
            scene_id: id.to_string(),
            text: text.to_string(),
        }
    }

    fn analyzers() -> AnalyzerVersions {
        AnalyzerVersions {
            visualner: "visualner".to_string(),
            semantic_core: "0.1.0".to_string(),
            phrase_impact: "phrase_impact".to_string(),
            lexicon: "lexicon.v1".to_string(),
        }
    }

    #[test]
    fn same_key_hits_and_different_text_misses() {
        let mut cache = ManifestCache::new(8);
        let scenes = vec![scene("s1", "Alberto Stasi e Andrea Sempio sono due nomi.")];
        let key = ManifestCache::key("it", &scenes, &analyzers());
        assert!(cache.get(&key).is_none(), "cold cache must miss");

        let manifest = manifest_with("it", &scenes);
        cache.insert(key.clone(), manifest.clone());

        let hit = cache.get(&key).expect("warm cache must hit");
        assert_eq!(hit.fingerprint, manifest.fingerprint);

        // Different text -> different fingerprint -> miss.
        let other = vec![scene("s1", "Testo completamente diverso.")];
        let other_key = ManifestCache::key("it", &other, &analyzers());
        assert_ne!(other_key, key);
        assert!(cache.get(&other_key).is_none());
    }

    #[test]
    fn analyzer_version_change_invalidates_the_key() {
        let scenes = vec![scene("s1", "Alberto Stasi è un nome.")];
        let v1 = AnalyzerVersions {
            visualner: "visualner".to_string(),
            semantic_core: "0.1.0".to_string(),
            phrase_impact: "phrase_impact".to_string(),
            lexicon: "lexicon.v1".to_string(),
        };
        let v2 = AnalyzerVersions {
            semantic_core: "0.2.0".to_string(),
            ..v1.clone()
        };
        let k1 = ManifestCache::key("it", &scenes, &v1);
        let k2 = ManifestCache::key("it", &scenes, &v2);
        assert_ne!(k1, k2, "a version bump must not serve stale results");
    }

    #[test]
    fn fifo_eviction_keeps_the_cache_bounded() {
        let mut cache = ManifestCache::new(2);
        let a = vec![scene("a", "Primo.")];
        let b = vec![scene("b", "Secondo.")];
        let c = vec![scene("c", "Terzo.")];
        let ka = ManifestCache::key("it", &a, &analyzers());
        let kb = ManifestCache::key("it", &b, &analyzers());
        let kc = ManifestCache::key("it", &c, &analyzers());
        cache.insert(ka.clone(), manifest_with("it", &a));
        cache.insert(kb.clone(), manifest_with("it", &b));
        cache.insert(kc, manifest_with("it", &c));
        assert_eq!(cache.len(), 2, "cap must be respected");
        assert!(cache.get(&ka).is_none(), "oldest entry evicted");
        assert!(cache.get(&kb).is_some());
    }

    #[test]
    fn zero_capacity_disables_caching() {
        let mut cache = ManifestCache::new(0);
        let scenes = vec![scene("s1", "Testo.")];
        let key = ManifestCache::key("it", &scenes, &analyzers());
        cache.insert(key.clone(), manifest_with("it", &scenes));
        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn editorial_inputs_participate_in_the_cache_key() {
        let scenes = vec![scene(
            "s1",
            "Il progetto contiene una prima fase e una verifica finale.",
        )];
        let mut lexical = ScriptRequest {
            language: "it".to_string(),
            scenes: scenes.clone(),
            editorial: None,
            entity_safety_cap: 0,
        };
        let baseline = ManifestCache::request_key(&lexical, &analyzers());
        lexical.editorial = Some(
            serde_json::from_value(serde_json::json!({
                "request": {
                    "transcript": "Il progetto contiene una prima fase e una verifica finale.",
                    "language": "it",
                    "embeddings": [],
                    "lexical_only": true
                },
                "scenes": []
            }))
            .expect("valid editorial request"),
        );
        let editorial = ManifestCache::request_key(&lexical, &analyzers());
        assert_ne!(
            baseline, editorial,
            "editorial output must never reuse a semantic-only cache entry"
        );
    }

    #[test]
    fn replay_returns_a_byte_identical_manifest() {
        let scenes = vec![scene("s1", "Il 42% contro il 18% è nel fascicolo.")];
        let first = manifest_with("it", &scenes);
        let mut cache = ManifestCache::new(4);
        let key = ManifestCache::key("it", &scenes, &analyzers());
        cache.insert(key.clone(), first.clone());
        let cached = cache.get(&key).unwrap();
        // The cached copy is byte-identical to a fresh analysis (idempotent).
        assert_eq!(
            serde_json::to_vec(&cached).unwrap(),
            serde_json::to_vec(&first).unwrap()
        );
    }
}
