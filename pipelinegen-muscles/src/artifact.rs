use crate::protocol::Response;

use crate::error::MuscleError;
use std::fs;
use std::path::Path;
use std::process::Command;

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
            let filename = format!("{stem}.part.{extension}");
            if parent.is_empty() {
                filename
            } else {
                Path::new(parent)
                    .join(filename)
                    .to_string_lossy()
                    .into_owned()
            }
        }
        None => format!("{final_path}.part"),
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

pub(crate) fn publish_output(part_path: &str, final_path: &str) -> Result<(), MuscleError> {
    let metadata = fs::metadata(part_path)
        .map_err(|error| MuscleError::generic(format!("output is missing: {error}")))?;
    if metadata.len() == 0 {
        let _ = fs::remove_file(part_path);
        return Err(MuscleError::generic("output is empty"));
    }
    fs::rename(part_path, final_path)
        .map_err(|error| MuscleError::generic(format!("publish output: {error}")))
}
