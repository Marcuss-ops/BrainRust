//! Build identity for the persistent manifest cache.
//!
//! The cache key cannot depend only on hand-written analyzer labels: an
//! uncommitted source change would otherwise reuse a manifest produced by a
//! different binary. This script hashes every Rust source file that can change
//! an analysis result, the relevant manifests, the workspace lockfile and the
//! git commit, and exposes the digest as `SEMANTIC_CACHE_BUILD_ID`.
//!
//! It runs at build time, so a rebuild after any edit in those directories
//! produces a different id without a manual bump.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// Directories whose sources can change an `analyze_script` result.
const SOURCE_DIRS: &[&str] = &[
    "src",
    "semantic-core/src",
    "visualner/src",
    "ndjson-stdio/src",
];

/// Build inputs that are not Rust sources but change the produced code.
const BUILD_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "semantic-engine/Cargo.toml",
    "semantic-core/Cargo.toml",
    "visualner/Cargo.toml",
    "ndjson-stdio/Cargo.toml",
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.clone());

    let mut files: Vec<PathBuf> = Vec::new();
    for dir in SOURCE_DIRS {
        collect_rust_files(&root.join(dir), &mut files);
    }
    for file in BUILD_FILES {
        files.push(root.join(file));
    }
    files.sort();
    files.dedup();

    let mut hasher = Sha256::new();
    for path in &files {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let relative = path.strip_prefix(&root).unwrap_or(path);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    hasher.update(git_head(&root).as_bytes());

    println!(
        "cargo:rustc-env=SEMANTIC_CACHE_BUILD_ID={}",
        lowercase_hex(&hasher.finalize())
    );
    for dir in SOURCE_DIRS {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    for file in BUILD_FILES {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }
    println!("cargo:rerun-if-changed=build.rs");
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn git_head(root: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
