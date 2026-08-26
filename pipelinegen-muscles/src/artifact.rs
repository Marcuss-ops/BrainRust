use crate::protocol::Response;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn failed_response(source_path: Option<String>, error: String) -> Response {
    Response {
        ok: false,
        operation: "cut_batch".to_string(),
        source_path,
        items: Vec::new(),
        metadata: None,
        error: Some(error),
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


/// sha256_file computes the lowercase SHA256 digest of a file using the same
/// `sha256sum` invocation the canonical render plan already relies on. It is
/// the hashing helper for the combined-audio render path so the Go adapter can
/// trust the digest Rust produced instead of re-hashing the output it just
/// rendered.
pub(crate) fn sha256_file(path: &str) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("compute SHA256 for {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!("compute SHA256 for {path} failed"));
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
        return Err(format!("sha256sum returned an invalid digest for {path}"));
    }
    Ok(digest)
}

pub(crate) fn publish_output(part_path: &str, final_path: &str) -> Result<(), String> {
    let metadata =
        fs::metadata(part_path).map_err(|error| format!("output is missing: {error}"))?;
    if metadata.len() == 0 {
        let _ = fs::remove_file(part_path);
        return Err("output is empty".to_string());
    }
    fs::rename(part_path, final_path).map_err(|error| {
        let _ = fs::remove_file(part_path);
        format!("publish output: {error}")
    })
}
