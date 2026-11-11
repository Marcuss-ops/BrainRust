//! In-memory manifest cache for the persistent worker.
//!
//! A persistent worker sees the same script text again on replay and on
//! repeated jobs. Re-running the extractor is pure CPU for identical input, so
//! the manifest is memoized under the request **fingerprint** (FNV-1a over the
//! canonical language, scenes, editorial options, safety cap and analyzer
//! versions). Equal inputs hit; any change to the text, editorial inputs or an
//! analyzer version yields a different fingerprint and misses, so the cache
//! can never serve a result computed by a different request or extractor build.

use std::collections::{HashMap, VecDeque};

use semantic_core::{AnalyzerVersions, ScriptRequest, SemanticManifest};

/// Default byte budget for the persistent worker. The entry cap still applies;
/// this budget bounds the total serialized manifest size.
pub const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;

struct CacheEntry {
    manifest: SemanticManifest,
    bytes: usize,
}

/// Bounded FIFO cache from request fingerprint to analyzed manifest.
///
/// FIFO (not LRU) keeps the eviction deterministic and the memory bounded: a
/// script batch is analysed once and replayed, so recency ordering adds book
/// keeping without changing which results are correct. The bound only stops a
/// long-lived worker from growing without limit; it never changes output.
pub struct ManifestCache {
    max_entries: usize,
    max_bytes: usize,
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
    total_bytes: usize,
    hits: u64,
    misses: u64,
}

impl ManifestCache {
    /// A cache holding at most `max_entries` manifests. Zero disables caching.
    pub fn new(max_entries: usize) -> Self {
        Self::with_budget(max_entries, DEFAULT_MAX_BYTES)
    }

    /// A cache with both an entry cap and a total serialized-byte budget.
    /// Zero for either cap disables caching.
    pub fn with_budget(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Request-complete key used by the worker, including editorial options,
    /// scene context and caller-supplied embedding vectors. This is the same
    /// fingerprint recorded on the manifest; there is no second, weaker key.
    pub fn request_key(request: &ScriptRequest, analyzers: &AnalyzerVersions) -> String {
        crate::request_fingerprint(request, analyzers)
    }

    /// Returns the cached manifest for `key`, if present.
    pub fn get(&mut self, key: &str) -> Option<SemanticManifest> {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return None;
        }
        match self.entries.get(key) {
            Some(entry) => {
                self.hits += 1;
                Some(entry.manifest.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Stores a manifest under `key`, evicting the oldest entries until both
    /// the entry cap and the serialized-byte budget hold. A manifest larger
    /// than the whole budget is not cached.
    pub fn insert(&mut self, key: String, manifest: SemanticManifest) {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return;
        }
        let bytes = serde_json::to_vec(&manifest)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        if bytes > self.max_bytes {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
            self.order.retain(|existing| existing != &key);
        }
        self.entries
            .insert(key.clone(), CacheEntry { manifest, bytes });
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.order.push_back(key);
        while self.entries.len() > self.max_entries || self.total_bytes > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Serialized bytes currently held by the cache.
    pub fn bytes(&self) -> usize {
        self.total_bytes
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

    fn request(language: &str, scenes: &[SceneInput]) -> ScriptRequest {
        ScriptRequest {
            language: language.to_string(),
            scenes: scenes.to_vec(),
            editorial: None,
            entity_safety_cap: 0,
        }
    }

    fn cache_key(language: &str, scenes: &[SceneInput]) -> String {
        ManifestCache::request_key(&request(language, scenes), &analyzers())
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
        let key = cache_key("it", &scenes);
        assert!(cache.get(&key).is_none(), "cold cache must miss");

        let manifest = manifest_with("it", &scenes);
        cache.insert(key.clone(), manifest.clone());

        let hit = cache.get(&key).expect("warm cache must hit");
        assert_eq!(hit.fingerprint, manifest.fingerprint);

        // Different text -> different fingerprint -> miss.
        let other = vec![scene("s1", "Testo completamente diverso.")];
        let other_key = cache_key("it", &other);
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
        let request = request("it", &scenes);
        let k1 = ManifestCache::request_key(&request, &v1);
        let k2 = ManifestCache::request_key(&request, &v2);
        assert_ne!(k1, k2, "a version bump must not serve stale results");
    }

    #[test]
    fn fifo_eviction_keeps_the_cache_bounded() {
        let mut cache = ManifestCache::new(2);
        let a = vec![scene("a", "Primo.")];
        let b = vec![scene("b", "Secondo.")];
        let c = vec![scene("c", "Terzo.")];
        let ka = cache_key("it", &a);
        let kb = cache_key("it", &b);
        let kc = cache_key("it", &c);
        cache.insert(ka.clone(), manifest_with("it", &a));
        cache.insert(kb.clone(), manifest_with("it", &b));
        cache.insert(kc, manifest_with("it", &c));
        assert_eq!(cache.len(), 2, "cap must be respected");
        assert!(cache.get(&ka).is_none(), "oldest entry evicted");
        assert!(cache.get(&kb).is_some());
    }

    #[test]
    fn byte_budget_evicts_old_entries_before_the_entry_cap() {
        let scenes_a = vec![scene("a", "Primo testo.")];
        let scenes_b = vec![scene("b", "Secondo testo.")];
        let manifest_a = manifest_with("it", &scenes_a);
        let manifest_b = manifest_with("it", &scenes_b);
        let size_a = serde_json::to_vec(&manifest_a).unwrap().len();
        let size_b = serde_json::to_vec(&manifest_b).unwrap().len();
        let budget = size_a.max(size_b) + 1;
        let mut cache = ManifestCache::with_budget(8, budget);
        let ka = cache_key("it", &scenes_a);
        let kb = cache_key("it", &scenes_b);
        cache.insert(ka.clone(), manifest_a);
        cache.insert(kb.clone(), manifest_b);
        assert_eq!(cache.len(), 1, "byte budget must evict before entry cap");
        assert!(cache.get(&ka).is_none());
        assert!(cache.get(&kb).is_some());
        assert!(cache.bytes() <= budget);
    }

    #[test]
    fn a_manifest_larger_than_the_byte_budget_is_not_cached() {
        let scenes = vec![scene("s1", "Testo.")];
        let key = cache_key("it", &scenes);
        let mut cache = ManifestCache::with_budget(8, 1);
        cache.insert(key.clone(), manifest_with("it", &scenes));
        assert!(cache.is_empty());
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn zero_capacity_disables_caching() {
        let mut cache = ManifestCache::new(0);
        let scenes = vec![scene("s1", "Testo.")];
        let key = cache_key("it", &scenes);
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
        let key = cache_key("it", &scenes);
        cache.insert(key.clone(), first.clone());
        let cached = cache.get(&key).unwrap();
        // The cached copy is byte-identical to a fresh analysis (idempotent).
        assert_eq!(
            serde_json::to_vec(&cached).unwrap(),
            serde_json::to_vec(&first).unwrap()
        );
    }
}
