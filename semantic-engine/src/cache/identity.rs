//! Build, content and model identity for the persistent cache key.
//!
//! The wire-visible analyzer labels are hand-written and a disk entry outlives
//! the build that wrote it. This identity adds the embedded lexicon content,
//! the build hash (sources, manifests and git commit) and the ICU major, so a
//! behaviour change without a label bump cannot serve a stale manifest. Model
//! identity (file name and SHA-256 content digest) is per language, so a
//! degraded worker and a worker with UDPipe models keep separate entries.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

/// Identity of the compiled pipeline for the persistent cache key.
pub(crate) fn cache_identity(language: &str) -> String {
    format!(
        "build={};content={};icu={};model={}",
        env!("SEMANTIC_CACHE_BUILD_ID"),
        semantic_core::content_fingerprint::embedded_content_fingerprint(),
        option_env!("RUST_ICU_MAJOR_VERSION_NUMBER").unwrap_or("unknown"),
        model_profile(language),
    )
}

/// UDPipe model identity for one language: the file name and SHA-256 of the
/// file content, or `absent` for the model-free fallback. The path itself is
/// not hashed because it differs per operator deployment; the content is what
/// changes the analysis.
fn model_profile(language: &str) -> String {
    let Some(path) = visualner::model_path(language) else {
        return "absent".to_string();
    };
    model_profile_for_path(&path)
}

fn model_profile_for_path(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match model_digest(path) {
        Ok(digest) => format!("{name}:{digest}"),
        Err(_) => format!("{name}:unknown"),
    }
}

/// SHA-256 of a model file, computed once per process and bounded to the same
/// sixteen slots as the loaded-model cache. The model is loaded once, so a
/// replaced file must not silently change the identity of the model in memory.
fn model_digest(path: &Path) -> std::io::Result<String> {
    const MODEL_DIGEST_CACHE_CAP: usize = 16;
    static DIGESTS: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let digests = DIGESTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = digests.lock() {
        if let Some(digest) = cache.get(path) {
            return Ok(digest.clone());
        }
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = semantic_core::text::lowercase_hex(hasher.finalize().as_slice());
    if let Ok(mut cache) = digests.lock() {
        if cache.len() >= MODEL_DIGEST_CACHE_CAP {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), digest.clone());
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn model_profile_detects_same_size_content_changes() {
        let root = temp_dir("model-profile");
        let first = root.join("first").join("model.udpipe");
        let second = root.join("second").join("model.udpipe");
        fs::create_dir_all(first.parent().expect("first parent")).expect("first dir");
        fs::create_dir_all(second.parent().expect("second parent")).expect("second dir");
        fs::write(&first, b"AAAA").expect("first model");
        fs::write(&second, b"BBBB").expect("second model");

        let first_profile = model_profile_for_path(&first);
        let second_profile = model_profile_for_path(&second);
        assert_eq!(
            fs::metadata(&first).expect("first metadata").len(),
            fs::metadata(&second).expect("second metadata").len()
        );
        assert_ne!(first_profile, second_profile);
        assert_eq!(first_profile, model_profile_for_path(&first));
        assert!(first_profile.starts_with("model.udpipe:"));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn missing_model_is_reported_as_unknown() {
        let root = temp_dir("model-missing");
        let path = root.join("absent.udpipe");
        assert_eq!(model_profile_for_path(&path), "absent.udpipe:unknown");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
