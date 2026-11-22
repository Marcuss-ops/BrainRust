//! Persistent manifest cache for the semantic worker.
//!
//! The in-memory [`ManifestCache`](super::ManifestCache) only survives as long
//! as the worker process. A batch that crashes, a worker restart or a replay on
//! the next day re-runs the extractor for text that did not change. This store
//! keeps the same manifests on disk, named by a content address.
//!
//! Key: SHA-256 over the canonical [`request_body`](crate::request_body) bytes
//! (language, scenes, editorial options, safety cap), the analyzer versions and
//! the [`cache_identity`](crate::cache_identity): embedded lexicon content,
//! build hash, ICU major and UDPipe model identity (name and size) for the
//! language. The
//! wire-visible `manifest.fingerprint` stays FNV-1a; widening the cache key
//! does not change the contract.
//!
//! The identity matters because a disk entry outlives the process that wrote
//! it: the hand-written analyzer labels alone would serve a manifest produced
//! by older data or an older binary.
//!
//! Every read is **fail-closed**: a truncated file, invalid JSON, a payload
//! written for another key, a different manifest schema, or a manifest whose
//! fingerprint does not match this request is a miss. A miss recomputes the
//! manifest; a corrupt entry is deleted so the next request does not pay for
//! decoding it again.
//!
//! Writes are atomic: bytes go to a unique `*.part` file in the same directory
//! and are renamed into place, so a reader never observes a partial manifest.
//! Eviction is by modification time, and a successful read refreshes that
//! stamp, which makes the bound least-recently-used rather than FIFO.
//!
//! The store is opt-in: [`DiskCache::from_env`] returns `None` unless
//! `SEMANTIC_CACHE_DIR` names a writable directory, so the default worker
//! behaviour is unchanged.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use semantic_core::{AnalyzerVersions, ScriptRequest, SemanticManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::request_body;

/// Directory that enables the persistent cache. Unset -> memory-only worker.
pub const CACHE_DIR_ENV: &str = "SEMANTIC_CACHE_DIR";
/// Optional byte budget for the directory. Defaults to [`DEFAULT_DISK_BYTES`].
pub const CACHE_BYTES_ENV: &str = "SEMANTIC_CACHE_MAX_BYTES";

/// Default bound for the on-disk cache: 1 GiB of serialized manifests. This is
/// a policy default, not a calibrated RSS target.
pub const DEFAULT_DISK_BYTES: u64 = 1024 * 1024 * 1024;

/// Bumped whenever the payload shape below changes. A different value is a
/// miss, so an older worker never decodes a newer file as if it were its own.
const CACHE_FORMAT: u32 = 2;
const KEY_PREFIX: &str = "manifest-";
/// Part files older than this are leftovers from a killed writer.
const STALE_PART_AGE: Duration = Duration::from_secs(60 * 60);
/// A full directory scan on every write does not scale past a few thousand
/// entries; the tracked byte count decides when a prune is actually needed,
/// and this periodic pass catches entries written by another process.
const PRUNE_EVERY_WRITES: u64 = 64;
/// A long-lived worker also prunes on a wall-clock interval, so a slow trickle
/// of writes cannot keep the directory above budget indefinitely.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// On-disk payload. `key`, `schema_version` and `manifest.fingerprint` are
/// re-checked on load; they are not trusted because the path matched.
#[derive(Serialize, Deserialize)]
struct Payload {
    cache_format: u32,
    key: String,
    schema_version: String,
    manifest: SemanticManifest,
}

/// A persistent, content-addressed manifest store rooted at one directory.
pub struct DiskCache {
    root: PathBuf,
    max_bytes: u64,
    /// Bytes this process believes are on disk. Approximate under concurrent
    /// writers; `prune` replaces it with the measured total.
    tracked_bytes: AtomicU64,
    writes: AtomicU64,
    prunes: AtomicU64,
    last_prune: Mutex<Instant>,
}

impl DiskCache {
    /// Opens the directory named by `SEMANTIC_CACHE_DIR`, if any.
    ///
    /// A missing variable disables the store. A directory that cannot be
    /// created disables it too: the worker keeps serving requests from memory
    /// and from fresh analysis rather than failing the job over a cache.
    pub fn from_env() -> Option<Self> {
        let root = std::env::var_os(CACHE_DIR_ENV)?;
        if root.is_empty() {
            return None;
        }
        let max_bytes = std::env::var(CACHE_BYTES_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
            .unwrap_or(DEFAULT_DISK_BYTES);
        Self::open(PathBuf::from(root), max_bytes).ok()
    }

    /// Opens (creating if needed) `root` and prunes it to `max_bytes`.
    pub fn open(root: PathBuf, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        let cache = Self {
            root,
            max_bytes,
            tracked_bytes: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            prunes: AtomicU64::new(0),
            last_prune: Mutex::new(Instant::now()),
        };
        cache.prune();
        Ok(cache)
    }

    /// Content address for a request. SHA-256 makes an accidental collision
    /// between two scripts impractical, which matters for a cache that a later
    /// process trusts without re-analysing.
    pub fn key(request: &ScriptRequest, analyzers: &AnalyzerVersions) -> String {
        Self::key_with_identity(
            request,
            analyzers,
            &crate::cache_identity(request.language.trim()),
        )
    }

    /// The key for an explicit identity. Production always goes through
    /// [`DiskCache::key`]; this entry point exists so a test can prove that a
    /// changed identity (a lexicon digest, a build, a runtime profile) moves
    /// the key without having to edit an embedded asset.
    pub fn key_with_identity(
        request: &ScriptRequest,
        analyzers: &AnalyzerVersions,
        identity: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(CACHE_FORMAT.to_le_bytes());
        // Build, embedded-content, ICU and model identity: the wire labels stay
        // untouched, but a behaviour change without a label bump still misses.
        hasher.update(identity.as_bytes());
        hasher.update(request_body(request, analyzers));
        format!("{KEY_PREFIX}{}", lowercase_hex(&hasher.finalize()))
    }

    /// Returns the stored manifest when every check holds, otherwise `None`.
    ///
    /// `fingerprint` is recomputed from the request by the caller; a mismatch
    /// means the file was written for different input and must not be served.
    pub fn load(&self, key: &str, fingerprint: &str) -> Option<Arc<SemanticManifest>> {
        let path = self.path(key);
        let bytes = fs::read(&path).ok()?;
        let payload: Payload = match serde_json::from_slice(&bytes) {
            Ok(payload) => payload,
            Err(_) => {
                // Fail closed and stop paying for the same corrupt bytes.
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        let valid = payload.cache_format == CACHE_FORMAT
            && payload.key == key
            && payload.schema_version == crate::SCHEMA_VERSION
            && payload.manifest.schema_version == payload.schema_version
            && payload.manifest.fingerprint == fingerprint;
        if !valid {
            return None;
        }
        touch(&path);
        Some(Arc::new(payload.manifest))
    }

    /// Writes one manifest atomically, then enforces the byte budget.
    ///
    /// Errors are swallowed: a full disk or a read-only directory degrades the
    /// cache, never the request. A manifest larger than the whole budget is not
    /// written, matching the in-memory rule.
    pub fn store(&self, key: &str, manifest: &SemanticManifest) {
        let Ok(encoded) = serde_json::to_vec(manifest) else {
            return;
        };
        self.store_bytes(key, &encoded);
    }

    /// Stores an already-serialized manifest under `key`.
    ///
    /// The payload wrapper is assembled around the manifest bytes instead of
    /// deserializing/rewriting them: the worker encodes the manifest once for
    /// both the memory budget and this store. The key and schema are small
    /// JSON strings produced by the serializer, never interpolated raw.
    pub fn store_bytes(&self, key: &str, encoded_manifest: &[u8]) {
        let (Ok(key_json), Ok(schema_json)) = (
            serde_json::to_string(key),
            serde_json::to_string(crate::SCHEMA_VERSION),
        ) else {
            return;
        };
        let mut bytes = Vec::with_capacity(encoded_manifest.len() + key_json.len() + 96);
        bytes.extend_from_slice(b"{\"cache_format\":");
        bytes.extend_from_slice(CACHE_FORMAT.to_string().as_bytes());
        bytes.extend_from_slice(b",\"key\":");
        bytes.extend_from_slice(key_json.as_bytes());
        bytes.extend_from_slice(b",\"schema_version\":");
        bytes.extend_from_slice(schema_json.as_bytes());
        bytes.extend_from_slice(b",\"manifest\":");
        bytes.extend_from_slice(encoded_manifest);
        bytes.push(b'}');
        if bytes.len() as u64 > self.max_bytes {
            return;
        }
        let part = self.part_path(key);
        if fs::write(&part, &bytes).is_err() {
            let _ = fs::remove_file(&part);
            return;
        }
        if fs::rename(&part, self.path(key)).is_err() {
            // Windows does not replace an existing destination with `rename`.
            // Removing first keeps the write atomic on POSIX and makes the
            // replacement possible elsewhere; a reader that loses the race just
            // reports a miss.
            let _ = fs::remove_file(self.path(key));
            if fs::rename(&part, self.path(key)).is_err() {
                let _ = fs::remove_file(&part);
                // Nothing was published: counting these bytes would make the
                // budget prune entries that are not actually there.
                return;
            }
        }
        let tracked = self
            .tracked_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed)
            + bytes.len() as u64;
        let writes = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
        let interval_elapsed = self
            .last_prune
            .lock()
            .map(|last| last.elapsed() >= PRUNE_INTERVAL)
            .unwrap_or(false);
        if tracked > self.max_bytes || writes.is_multiple_of(PRUNE_EVERY_WRITES) || interval_elapsed
        {
            self.prune();
        }
    }

    /// Number of directory scans performed, including the one at open.
    pub fn prune_count(&self) -> u64 {
        self.prunes.load(Ordering::Relaxed)
    }

    /// Deletes the oldest entries until the directory fits `max_bytes`, and
    /// removes `.part` files abandoned by a killed writer.
    pub fn prune(&self) {
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
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part") {
                let stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .map(|age| age > STALE_PART_AGE)
                    .unwrap_or(false);
                if stale {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            if !name.starts_with(KEY_PREFIX) || !name.ends_with(".json") {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((path, metadata.len(), modified));
        }

        let mut total: u64 = files.iter().map(|(_, bytes, _)| *bytes).sum();
        if total > self.max_bytes {
            files.sort_by_key(|entry| entry.2);
            for (path, bytes, _) in files {
                if total <= self.max_bytes {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    total = total.saturating_sub(bytes);
                }
            }
        }
        self.tracked_bytes.store(total, Ordering::Relaxed);
        self.prunes.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut last) = self.last_prune.lock() {
            *last = Instant::now();
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }

    fn part_path(&self, key: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        self.root
            .join(format!(".{key}.{}.{nanos}.part", std::process::id()))
    }
}

/// Refreshes mtime so eviction is least-recently-used. Best effort: a cache on
/// a read-only mount still serves hits, it just evicts by write time.
fn touch(path: &Path) {
    let Ok(file) = fs::OpenOptions::new().write(true).open(path) else {
        return;
    };
    let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_core::SceneInput;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "semantic-cache-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp cache dir");
        root
    }

    fn request(text: &str) -> ScriptRequest {
        ScriptRequest {
            language: "it".to_string(),
            scenes: vec![SceneInput {
                scene_id: "s1".to_string(),
                text: text.to_string(),
            }],
            editorial: None,
            entity_safety_cap: 0,

            reference_date: None,
        }
    }

    fn manifest(text: &str) -> SemanticManifest {
        crate::analyze_script(&request(text)).expect("analysis succeeds")
    }

    fn key_for(text: &str) -> String {
        DiskCache::key(&request(text), &crate::analyzer_versions())
    }

    #[test]
    fn store_then_load_returns_a_byte_identical_manifest() {
        let root = temp_root("roundtrip");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Alberto Stasi e Andrea Sempio sono due nomi.");
        let key = key_for("Alberto Stasi e Andrea Sempio sono due nomi.");

        cache.store(&key, &manifest);
        let loaded = cache
            .load(&key, &manifest.fingerprint)
            .expect("stored entry loads");

        assert_eq!(
            serde_json::to_vec(loaded.as_ref()).unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
            "a disk hit must be byte-identical to a fresh analysis"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pre_encoded_manifest_uses_the_same_payload_format() {
        let root = temp_root("pre-encoded");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let text = "Il comune di Garlasco ha 42 abitanti.";
        let manifest = manifest(text);
        let key = key_for(text);
        let encoded = serde_json::to_vec(&manifest).expect("encode manifest");

        cache.store_bytes(&key, &encoded);
        let loaded = cache
            .load(&key, &manifest.fingerprint)
            .expect("pre-encoded entry loads");

        assert_eq!(
            serde_json::to_vec(loaded.as_ref()).unwrap(),
            encoded,
            "the wrapper must not rewrite the manifest bytes"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_corrupt_entry_is_a_miss_and_is_removed() {
        let root = temp_root("corrupt");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Testo.");
        let key = key_for("Testo.");
        cache.store(&key, &manifest);

        fs::write(cache.path(&key), b"{\"cache_format\":1,").expect("truncate");
        assert!(cache.load(&key, &manifest.fingerprint).is_none());
        assert!(
            !cache.path(&key).exists(),
            "a corrupt entry is deleted, not retried forever"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_payload_written_for_another_key_is_a_miss() {
        let root = temp_root("foreign-key");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Primo testo.");
        let first = key_for("Primo testo.");
        let second = key_for("Secondo testo.");
        cache.store(&first, &manifest);

        fs::copy(cache.path(&first), cache.path(&second)).expect("copy entry");
        assert!(cache.load(&second, &manifest.fingerprint).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_manifest_from_another_request_is_a_miss_even_with_a_matching_key() {
        let root = temp_root("fingerprint");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Primo testo.");
        let key = key_for("Primo testo.");
        cache.store(&key, &manifest);

        assert!(
            cache.load(&key, "0000000000000000").is_none(),
            "the manifest fingerprint must match the request being served"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn different_text_uses_a_different_key() {
        assert_ne!(key_for("Primo testo."), key_for("Secondo testo."));
    }

    #[test]
    fn the_key_changes_when_the_injected_identity_changes() {
        let request = request("Testo.");
        let analyzers = crate::analyzer_versions();
        let first = DiskCache::key_with_identity(&request, &analyzers, "content=aaa");
        let second = DiskCache::key_with_identity(&request, &analyzers, "content=bbb");
        let repeated = DiskCache::key_with_identity(&request, &analyzers, "content=aaa");
        assert_ne!(first, second, "an identity change must move the key");
        assert_eq!(first, repeated, "the same identity is stable");
        assert_eq!(DiskCache::key(&request, &analyzers), {
            DiskCache::key_with_identity(
                &request,
                &analyzers,
                &crate::cache_identity(request.language.trim()),
            )
        });
    }

    #[test]
    fn prune_is_throttled_between_stores() {
        let root = temp_root("throttle");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        assert_eq!(cache.prune_count(), 1, "open prunes once");
        let manifest = manifest("Testo.");
        for index in 0..100 {
            cache.store(&format!("manifest-{index:064x}"), &manifest);
        }
        assert_eq!(
            cache.prune_count(),
            2,
            "only the 64th write triggers a directory scan"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn two_thousand_small_entries_do_not_scan_on_every_store() {
        let root = temp_root("many-entries");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Testo piccolo.");
        for index in 0..2_000 {
            cache.store(&format!("manifest-{index:064x}"), &manifest);
        }
        assert!(
            cache.prune_count() <= 2_000 / PRUNE_EVERY_WRITES + 2,
            "prune scans {} times for 2,000 stores",
            cache.prune_count()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_key_carries_the_build_and_content_identity() {
        let identity = crate::cache_identity("it");
        assert!(identity.starts_with("build="), "got {identity}");
        assert!(identity.contains(";icu="), "got {identity}");
        assert!(identity.contains(";model="), "got {identity}");
        let content = identity
            .split("content=")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .expect("content fingerprint present");
        assert_eq!(content.len(), 64, "content fingerprint is a SHA-256 hex");
        assert!(content.bytes().all(|byte| byte.is_ascii_hexdigit()));

        let key = key_for("Testo.");
        assert_eq!(key.len(), "manifest-".len() + 64);
    }

    #[test]
    fn an_unknown_key_is_a_miss() {
        let root = temp_root("unknown");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        assert!(cache
            .load(&key_for("Mai scritto."), "0000000000000000")
            .is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_successful_store_leaves_no_part_file_behind() {
        let root = temp_root("atomic");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let manifest = manifest("Testo atomico.");
        cache.store(&key_for("Testo atomico."), &manifest);

        let parts: Vec<_> = fs::read_dir(&root)
            .expect("dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(parts.is_empty(), "atomic publish must rename the part file");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_evicts_the_oldest_entries_until_the_budget_holds() {
        let root = temp_root("prune");

        let texts = ["Primo testo.", "Secondo testo.", "Terzo testo."];
        let mut manifests = Vec::new();
        for text in texts {
            let manifest = manifest(text);
            let size = serde_json::to_vec(&Payload {
                cache_format: CACHE_FORMAT,
                key: key_for(text),
                schema_version: manifest.schema_version.clone(),
                manifest: manifest.clone(),
            })
            .unwrap()
            .len() as u64;
            manifests.push((text, manifest, size));
        }
        // Exactly the two newest entries fit, so eviction has one unambiguous
        // target no matter how the manifest sizes order.
        let budget = manifests[1].2 + manifests[2].2;
        let cache = DiskCache::open(root.clone(), budget).expect("reopen with budget");
        for (text, manifest, _) in &manifests {
            cache.store(&key_for(text), manifest);
            // Distinct mtimes so the eviction order is unambiguous.
            std::thread::sleep(Duration::from_millis(10));
        }

        let present: Vec<&str> = texts
            .iter()
            .copied()
            .filter(|text| cache.path(&key_for(text)).exists())
            .collect();
        assert_eq!(present, vec!["Secondo testo.", "Terzo testo."]);
        let total: u64 = fs::read_dir(&root)
            .expect("dir")
            .flatten()
            .map(|entry| entry.metadata().map(|meta| meta.len()).unwrap_or(0))
            .sum();
        assert!(total <= budget, "the directory must stay under the budget");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_stale_part_file_is_removed_by_prune() {
        let root = temp_root("stale-part");
        let cache = DiskCache::open(root.clone(), DEFAULT_DISK_BYTES).expect("open");
        let part = root.join(".manifest-abandoned.999.1.part");
        fs::write(&part, b"partial").expect("write part");
        let old = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&part)
            .expect("open part");
        file.set_times(fs::FileTimes::new().set_modified(old))
            .expect("age part file");

        cache.prune();
        assert!(!part.exists(), "a killed writer must not leak part files");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_manifest_larger_than_the_budget_is_not_written() {
        let root = temp_root("oversized");
        let cache = DiskCache::open(root.clone(), 1).expect("open");
        let manifest = manifest("Testo troppo grande per il budget.");
        cache.store(&key_for("Testo troppo grande per il budget."), &manifest);
        assert!(fs::read_dir(&root).expect("dir").flatten().next().is_none());
        let _ = fs::remove_dir_all(root);
    }
}
