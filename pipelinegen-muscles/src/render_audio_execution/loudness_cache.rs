//! Static BGM loudness measurements, cached across renders.
//!
//! `measure_mean_volume_db` decodes the whole music asset with `volumedetect`
//! before the mix starts. Measured on the production assets that is 4-7 s for a
//! 3-6 minute track, and it is pure source preparation: the same file always
//! measures the same mean volume. On repeated jobs the asset is materialized
//! again under the same content-addressed name, so the measurement is repeated
//! for nothing.
//!
//! The cache key is the file identity — path, size and mtime in nanoseconds —
//! and every entry repeats that identity, so a hit is only served when the file
//! on disk is bit-for-bit the one that was measured as far as the filesystem
//! can tell us. A collision in the (non-cryptographic) name hash is therefore a
//! miss, never a wrong value: the entry is validated after parsing.
//!
//! Disabled by setting `PIPELINE_MUSCLES_CACHE_DIR=` (empty). Otherwise the
//! directory is `PIPELINE_MUSCLES_CACHE_DIR` or `<tmp>/pipelinegen-muscles`.
//! Writes are best effort and atomic (temp file + rename): a cache failure can
//! never fail a render, and concurrent workers writing the same entry is safe.
//!
//! Only a measured, finite mean volume is ever stored. A failed measurement
//! (decoder error, timeout, no `mean_volume` line, digital silence reported as
//! `-inf`) is a property of that attempt, not of the file: caching it would turn
//! one transient failure into a permanent one.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

const ENTRY_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u32,
    path: String,
    size: u64,
    mtime_ns: u64,
    mean_db: f64,
}

/// File identity the cached measurement belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    mtime_ns: u64,
}

pub(super) struct LoudnessCache {
    dir: Option<PathBuf>,
}

impl LoudnessCache {
    /// Cache configured by the worker environment. The Go side owns the
    /// process, so the directory (when set) is absolute.
    pub(super) fn from_env() -> Self {
        let configured = std::env::var("PIPELINE_MUSCLES_CACHE_DIR").ok();
        let dir = match configured.as_deref() {
            Some("") => None,
            Some(value) => Some(PathBuf::from(value)),
            None => Some(std::env::temp_dir().join("pipelinegen-muscles")),
        };
        Self { dir }
    }

    #[cfg(test)]
    pub(super) fn at(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    /// Cached measurement, or `None` when the entry is absent, unreadable,
    /// corrupt, from another entry version, or belongs to a different file.
    pub(super) fn load(&self, path: &str) -> Option<f64> {
        let (entry_path, fingerprint) = self.key(path)?;
        let raw = fs::read_to_string(entry_path).ok()?;
        let entry: Entry = serde_json::from_str(&raw).ok()?;
        if entry.version != ENTRY_VERSION
            || entry.path != path
            || entry.size != fingerprint.size
            || entry.mtime_ns != fingerprint.mtime_ns
            || !entry.mean_db.is_finite()
        {
            return None;
        }
        Some(entry.mean_db)
    }

    /// Stores a measurement. Best effort: every failure path is silent because
    /// the cache is an optimization, never a correctness dependency.
    pub(super) fn store(&self, path: &str, mean_db: f64) {
        if !mean_db.is_finite() {
            return;
        }
        let Some((entry_path, fingerprint)) = self.key(path) else {
            return;
        };
        let Some(dir) = entry_path.parent() else {
            return;
        };
        let entry = Entry {
            version: ENTRY_VERSION,
            path: path.to_owned(),
            size: fingerprint.size,
            mtime_ns: fingerprint.mtime_ns,
            mean_db,
        };
        let Ok(body) = serde_json::to_vec(&entry) else {
            return;
        };
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let temp =
            entry_path.with_extension(format!("tmp{}-{}", std::process::id(), unique_suffix()));
        if fs::write(&temp, &body).is_ok() && fs::rename(&temp, &entry_path).is_err() {
            let _ = fs::remove_file(&temp);
        }
    }

    fn key(&self, path: &str) -> Option<(PathBuf, Fingerprint)> {
        let dir = self.dir.as_ref()?;
        Some((dir.join(entry_name(path)?), fingerprint(path)?))
    }
}

/// Entry file name: a non-cryptographic 64-bit hash keeps the name short while
/// the identity inside the entry keeps a collision harmless.
fn entry_name(path: &str) -> Option<String> {
    let fingerprint = fingerprint(path)?;
    Some(format!(
        "{:016x}-{}-{}.json",
        fnv1a64(path.as_bytes()),
        fingerprint.size,
        fingerprint.mtime_ns
    ))
}

fn fingerprint(path: &str) -> Option<Fingerprint> {
    let metadata = fs::metadata(Path::new(path)).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let since_epoch = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(Fingerprint {
        size: metadata.len(),
        mtime_ns: u64::try_from(since_epoch.as_nanos()).ok()?,
    })
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "pipelinegen-loudness-cache-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }

        fn asset(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.0.join(name);
            fs::write(&path, bytes).expect("write asset");
            path.to_string_lossy().into_owned()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn round_trips_a_measurement() {
        let temp = TempDir::new();
        let path = temp.asset("bgm.mp3", b"not really audio");
        let cache = LoudnessCache::at(temp.0.join("cache"));
        assert_eq!(cache.load(&path), None);
        cache.store(&path, -20.5);
        assert_eq!(cache.load(&path), Some(-20.5));
    }

    #[test]
    fn a_non_finite_measurement_is_never_stored() {
        let temp = TempDir::new();
        let path = temp.asset("silence.mp3", b"silence");
        let cache_dir = temp.0.join("cache");
        let cache = LoudnessCache::at(cache_dir.clone());
        // A failed/silent measurement is a property of the attempt, not the
        // file: caching it would make one transient failure permanent.
        cache.store(&path, f64::NEG_INFINITY);
        cache.store(&path, f64::NAN);
        assert_eq!(cache.load(&path), None);
        let entries = fs::read_dir(&cache_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(entries, 0);
    }

    #[test]
    fn a_different_size_or_mtime_is_a_miss() {
        let temp = TempDir::new();
        let path = temp.asset("bgm.mp3", b"first version");
        let cache = LoudnessCache::at(temp.0.join("cache"));
        cache.store(&path, -18.0);
        assert_eq!(cache.load(&path), Some(-18.0));

        // Same length, new mtime: the identity changed.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&path, b"second versio").expect("rewrite same length");
        assert_eq!(cache.load(&path), None, "mtime change must invalidate");

        // New length, mtime irrelevant.
        fs::write(&path, b"third").expect("rewrite shorter");
        assert_eq!(cache.load(&path), None, "size change must invalidate");
    }

    #[test]
    fn a_corrupt_or_foreign_entry_is_a_miss() {
        let temp = TempDir::new();
        let path = temp.asset("bgm.mp3", b"audio");
        let cache = LoudnessCache::at(temp.0.join("cache"));
        cache.store(&path, -21.0);
        let (entry, _) = cache.key(&path).expect("entry path");

        fs::write(&entry, b"{not json").expect("corrupt entry");
        assert_eq!(cache.load(&path), None);

        // An entry copied under another file's name carries the wrong path and
        // is rejected even though the name matched a real asset.
        let other = temp.asset("other.mp3", b"other audio");
        cache.store(&other, -12.0);
        let (other_entry, _) = cache.key(&other).expect("other entry");
        fs::copy(&other_entry, &entry).expect("copy entry");
        assert_eq!(cache.load(&path), None, "entry identity must be verified");

        // A newer entry version is ignored instead of misread.
        let mut entry_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&other_entry).expect("entry")).expect("json");
        entry_json["version"] = serde_json::json!(ENTRY_VERSION + 1);
        fs::write(&entry, serde_json::to_vec(&entry_json).expect("encode")).expect("write");
        assert_eq!(cache.load(&path), None);
    }

    #[test]
    fn a_missing_file_is_never_cached() {
        let temp = TempDir::new();
        let missing = temp.0.join("absent.mp3").to_string_lossy().into_owned();
        let cache = LoudnessCache::at(temp.0.join("cache"));
        cache.store(&missing, -10.0);
        assert_eq!(cache.load(&missing), None);
        let cache_dir = temp.0.join("cache");
        let entries = fs::read_dir(&cache_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(entries, 0, "an unreadable asset must not produce an entry");
    }

    #[test]
    fn an_unwritable_cache_is_silent() {
        let temp = TempDir::new();
        let path = temp.asset("bgm.mp3", b"audio");
        // A regular file where the directory should be.
        let blocked = temp.0.join("blocked");
        fs::write(&blocked, b"file").expect("blocker");
        let cache = LoudnessCache::at(blocked);
        cache.store(&path, -19.0);
        assert_eq!(cache.load(&path), None);
    }
}
