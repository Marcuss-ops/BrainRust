//! Per-scene cache for the pure phase of script analysis.
//!
//! Phase 1 is scene-local and independent of the cross-scene identity registry:
//! UDPipe/tokenisation, local NER, dates/numbers/places and the retained
//! temporal facts. It is the expensive half of `analyze_script`, and the same
//! scene text appears in a replay or in two jobs that share a paragraph.
//!
//! The key is SHA-256 over the scene text, the language and the analyzer
//! versions, widened with the same build/content/model identity as the manifest
//! cache. A disk entry outlives the process that wrote it, so a change to an
//! embedded lexicon, a model file or the binary must miss even when the
//! hand-written analyzer labels did not move.
//!
//! Phase 2 (identity registration, coreference, discourse, linker, visual
//! consolidation) is deliberately NOT cached here: it depends on the registry
//! built from every earlier scene, so caching it per scene could leak
//! cross-script coreference.
//!
//! Reads are fail-closed: a truncated file, a payload for another key or
//! language, or an unknown format is a miss and the entry is removed. Writes
//! are atomic (`*.part` then rename), so a concurrent reader never observes a
//! partial scene.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semantic_core::text::lowercase_hex;
use semantic_core::{AnalyzerVersions, SentenceSpan, TemporalFact};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cache_identity;
use super::disk::CACHE_DIR_ENV;

/// Bumped whenever the scene payload shape changes.
const SCENE_CACHE_FORMAT: u32 = 1;
const KEY_PREFIX: &str = "scene-";
/// Optional byte budget for the scene directory.
pub const SCENE_CACHE_BYTES_ENV: &str = "SEMANTIC_SCENE_CACHE_MAX_BYTES";
/// Default bound: 512 MiB of serialized scene drafts.
pub const DEFAULT_SCENE_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const PRUNE_EVERY_WRITES: u64 = 64;
const STALE_PART_AGE: Duration = Duration::from_secs(60 * 60);

/// Phase-1 output for one scene. `analysis.tokens` carries the token stream
/// phase 2 needs; the cache stores it beside the analysis because the wire
/// `Analysis` serializer deliberately skips tokens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalSceneDraft {
    pub scene_id: String,
    pub analysis: visualner::Analysis,
    pub temporal_facts: Vec<TemporalFact>,
    pub sentence_spans: Vec<SentenceSpan>,
}

#[derive(Serialize)]
struct PayloadRef<'a> {
    cache_format: u32,
    key: &'a str,
    language: &'a str,
    draft: &'a LocalSceneDraft,
    tokens: &'a [visualner::AnalyzedToken],
}

#[derive(Deserialize)]
struct Payload {
    cache_format: u32,
    key: String,
    language: String,
    draft: LocalSceneDraft,
    tokens: Vec<visualner::AnalyzedToken>,
}

/// Content-addressed scene store rooted at `<SEMANTIC_CACHE_DIR>/scenes`.
pub struct SceneCache {
    root: PathBuf,
    max_bytes: u64,
    writes: AtomicU64,
}

impl SceneCache {
    /// Opens the scene cache under the same directory as the manifest cache.
    /// Unset or unusable configuration disables it; analysis never fails over a
    /// cache.
    pub fn from_env() -> Option<Self> {
        let root = std::env::var_os(CACHE_DIR_ENV)?;
        if root.is_empty() {
            return None;
        }
        let max_bytes = std::env::var(SCENE_CACHE_BYTES_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
            .unwrap_or(DEFAULT_SCENE_CACHE_BYTES);
        Self::open(PathBuf::from(root).join("scenes"), max_bytes).ok()
    }

    /// Opens (creating if needed) `root` and prunes it to `max_bytes`.
    pub fn open(root: PathBuf, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        let cache = Self {
            root,
            max_bytes,
            writes: AtomicU64::new(0),
        };
        cache.prune();
        Ok(cache)
    }

    /// SHA-256 over text + language + analyzer versions + build/model identity.
    pub fn key(text: &str, language: &str, analyzers: &AnalyzerVersions) -> String {
        let mut hasher = Sha256::new();
        hasher.update(SCENE_CACHE_FORMAT.to_le_bytes());
        hasher.update(language.as_bytes());
        hasher.update([0u8]);
        hasher.update(text.as_bytes());
        hasher.update([0u8]);
        // `AnalyzerVersions` has a stable serde shape; the JSON bytes are only
        // an internal key component, never a wire value.
        hasher.update(serde_json::to_vec(analyzers).unwrap_or_default());
        hasher.update([0u8]);
        hasher.update(cache_identity(language).as_bytes());
        format!("{KEY_PREFIX}{}", lowercase_hex(&hasher.finalize()))
    }

    /// Returns the stored draft when every check holds, otherwise `None`.
    pub fn load(&self, key: &str, language: &str) -> Option<LocalSceneDraft> {
        let path = self.path(key);
        let bytes = fs::read(&path).ok()?;
        let payload: Payload = match serde_json::from_slice(&bytes) {
            Ok(payload) => payload,
            Err(_) => {
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        if payload.cache_format != SCENE_CACHE_FORMAT
            || payload.key != key
            || payload.language != language
        {
            return None;
        }
        let mut draft = payload.draft;
        draft.analysis.tokens = payload.tokens;
        visit(&path);
        Some(draft)
    }

    /// Writes one scene atomically, then enforces the byte budget periodically.
    /// Errors degrade the cache, never the request.
    pub fn store(&self, key: &str, language: &str, draft: &LocalSceneDraft) {
        let payload = PayloadRef {
            cache_format: SCENE_CACHE_FORMAT,
            key,
            language,
            draft,
            tokens: &draft.analysis.tokens,
        };
        let Ok(encoded) = serde_json::to_vec(&payload) else {
            return;
        };
        let path = self.path(key);
        if self.write_atomic(&path, &encoded) {
            let writes = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
            if writes.is_multiple_of(PRUNE_EVERY_WRITES) {
                self.prune();
            }
        }
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> bool {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let part = self.root.join(format!(
            ".{}.{}.{}.part",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::process::id(),
            nanos
        ));
        if fs::write(&part, bytes).is_err() {
            let _ = fs::remove_file(&part);
            return false;
        }
        if fs::rename(&part, path).is_err() {
            let _ = fs::remove_file(&part);
            return false;
        }
        true
    }

    fn path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }

    /// Removes stale part files and the oldest entries until the byte budget
    /// holds. A failed scan keeps the evidence already on disk.
    fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        let now = SystemTime::now();
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.ends_with(".part") {
                let stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age > STALE_PART_AGE);
                if stale {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            if !name.starts_with(KEY_PREFIX) {
                continue;
            }
            files.push((
                path,
                metadata.len(),
                metadata.modified().unwrap_or(UNIX_EPOCH),
            ));
        }
        let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

/// Refresh the modification stamp so `prune` evicts by least-recent use.
fn visit(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use visualner::AnalyzedToken;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("brainrust-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn draft(scene_id: &str) -> LocalSceneDraft {
        LocalSceneDraft {
            scene_id: scene_id.to_string(),
            analysis: visualner::Analysis {
                entities: Vec::new(),
                tokens: vec![AnalyzedToken {
                    text: "alpha".to_string(),
                    lemma: None,
                    upos: None,
                    features: None,
                    dependency: None,
                    id: None,
                    head: None,
                    sentence_id: None,
                    start: 0,
                    end: 5,
                }],
                relations: Vec::new(),
                lower_thirds: Vec::new(),
                bullets: Vec::new(),
                chart_points: Vec::new(),
                temporal_facts: Vec::new(),
                numeric_facts: Vec::new(),
            },
            temporal_facts: Vec::new(),
            sentence_spans: vec![SentenceSpan {
                start_byte: 0,
                end_byte: 5,
            }],
        }
    }

    #[test]
    fn scene_key_changes_with_text_language_and_analyzers() {
        let analyzers = crate::analyzer_versions();
        let base = SceneCache::key("alpha", "it", &analyzers);
        assert_ne!(base, SceneCache::key("alpha beta", "it", &analyzers));
        assert_ne!(base, SceneCache::key("alpha", "en", &analyzers));
        let mut changed = analyzers.clone();
        changed.visualner = "visualner.changed".to_string();
        assert_ne!(base, SceneCache::key("alpha", "it", &changed));
    }

    #[test]
    fn scene_round_trip_preserves_tokens_and_rejects_wrong_language() {
        let root = temp_dir("scene-cache");
        let cache = SceneCache::open(root.clone(), 1 << 20).expect("cache");
        let analyzers = crate::analyzer_versions();
        let key = SceneCache::key("alpha", "it", &analyzers);
        let stored = draft("s1");
        cache.store(&key, "it", &stored);
        let loaded = cache.load(&key, "it").expect("hit");
        assert_eq!(loaded.analysis.tokens, stored.analysis.tokens);
        assert_eq!(loaded.sentence_spans, stored.sentence_spans);
        assert!(cache.load(&key, "en").is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn changed_key_is_a_miss() {
        let root = temp_dir("scene-cache-miss");
        let cache = SceneCache::open(root.clone(), 1 << 20).expect("cache");
        let analyzers = crate::analyzer_versions();
        let key = SceneCache::key("alpha", "it", &analyzers);
        cache.store(&key, "it", &draft("s1"));
        assert!(cache
            .load(&SceneCache::key("beta", "it", &analyzers), "it")
            .is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
