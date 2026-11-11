use crate::protocol::Response;

use crate::error::MuscleError;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Distinguishes concurrent renders that target the same final path. A
/// deterministic `.part` name lets two workers write (and rename) the same
/// temporary file; a unique suffix keeps every publish atomic and isolated.
static NEXT_PART: AtomicU64 = AtomicU64::new(0);

pub(crate) fn failed_response(source_path: Option<String>, error: impl ToString) -> Response {
    Response {
        ok: false,
        operation: "cut_batch".to_string(),
        source_path,
        items: Vec::new(),
        metadata: None,
        error: Some(error.to_string()),
    }
}

pub(crate) fn part_path(final_path: &str) -> String {
    let unique = format!(
        "{}.{}",
        std::process::id(),
        NEXT_PART.fetch_add(1, Ordering::Relaxed)
    );
    let path = Path::new(final_path);
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => {
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let parent = path
                .parent()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let filename = format!("{stem}.{unique}.part.{extension}");
            if parent.is_empty() {
                filename
            } else {
                Path::new(parent)
                    .join(filename)
                    .to_string_lossy()
                    .into_owned()
            }
        }
        None => format!("{final_path}.{unique}.part"),
    }
}

/// sha256_file is the crate-wide canonical file hasher: it computes the
/// lowercase SHA256 digest of a file using the same `sha256sum` invocation the
/// canonical render plan relies on. Every consumer (combined-audio render
/// hashing, clip-plan artifact validation) routes through this single
/// implementation so the digest contract has exactly one owner.
pub(crate) fn sha256_file(path: &str) -> Result<String, MuscleError> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for {path}: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "compute SHA256 for {path} failed"
        )));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if digest.len() != 64
        || !digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(MuscleError::generic(format!(
            "sha256sum returned an invalid digest for {path}"
        )));
    }
    Ok(digest)
}

/// Latest generation published per final path in this worker process. The
/// generation is assigned when `part_path` is created, so a slow older render
/// cannot replace a newer completed one. It is process-local by design: the
/// worker is the unit that owns a final output path.
fn latest_published() -> &'static Mutex<HashMap<String, u64>> {
    static LATEST: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LATEST.get_or_init(|| Mutex::new(HashMap::new()))
}

fn part_generation(part_path: &str) -> Option<u64> {
    let file = Path::new(part_path).file_name()?.to_str()?;
    let (prefix, _) = file.rsplit_once(".part")?;
    prefix.rsplit('.').next()?.parse().ok()
}

pub(crate) fn publish_output(part_path: &str, final_path: &str) -> Result<(), MuscleError> {
    let metadata = fs::metadata(part_path)
        .map_err(|error| MuscleError::generic(format!("output is missing: {error}")))?;
    if metadata.len() == 0 {
        let _ = fs::remove_file(part_path);
        return Err(MuscleError::generic("output is empty"));
    }
    let generation = part_generation(part_path);
    let mut latest = latest_published()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(generation) = generation {
        if let Some(previous) = latest.get(final_path) {
            if generation < *previous {
                let _ = fs::remove_file(part_path);
                return Err(MuscleError::generic(format!(
                    "stale output attempt {generation} cannot replace generation {previous}"
                )));
            }
        }
    }
    fs::rename(part_path, final_path)
        .map_err(|error| MuscleError::generic(format!("publish output: {error}")))?;
    if let Some(generation) = generation {
        latest.insert(final_path.to_string(), generation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{part_path, publish_output};
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn part_paths_are_unique_for_the_same_final_path() {
        let paths: HashSet<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..32)
                .map(|_| scope.spawn(|| part_path("/tmp/final.mp4")))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("part path worker"))
                .collect()
        });
        assert_eq!(paths.len(), 32);
    }

    #[test]
    fn part_path_keeps_the_parent_and_extension() {
        let path = part_path("/tmp/final.mp4");
        assert!(path.starts_with("/tmp/final."));
        assert!(path.ends_with(".part.mp4"));
    }

    #[test]
    fn part_path_handles_a_path_without_extension() {
        let path = part_path("/tmp/final");
        assert!(path.starts_with("/tmp/final."));
        assert!(path.ends_with(".part"));
    }

    #[test]
    fn concurrent_publishes_leave_one_complete_file() {
        let base = std::env::temp_dir().join(format!(
            "brainrust-publish-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("temp dir");
        let final_path = base.join("final.mp4");
        let final_path = final_path.to_string_lossy().into_owned();
        let first = part_path(&final_path);
        let second = part_path(&final_path);
        fs::write(&first, b"first-complete").expect("first part");
        fs::write(&second, b"second-complete").expect("second part");
        let (first_result, second_result) = std::thread::scope(|scope| {
            let a = scope.spawn(|| publish_output(&first, &final_path));
            let b = scope.spawn(|| publish_output(&second, &final_path));
            (
                a.join().expect("first publisher"),
                b.join().expect("second publisher"),
            )
        });
        assert!(
            first_result.is_ok() || second_result.is_ok(),
            "at least one attempt must publish"
        );
        assert!(second_result.is_ok(), "the newest attempt must publish");
        let published = fs::read(&final_path).expect("published output");
        assert_eq!(
            published, b"second-complete",
            "the final file must be the newest complete attempt, never a mix"
        );
        let _ = fs::remove_dir_all(&base);
    }
}
