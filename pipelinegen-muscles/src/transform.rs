use crate::artifact::{failed_response, publish_output};
use crate::protocol::{MediaMetadata, Operation, Request, Response};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes concurrent concat lists written by the same process. The PID
/// alone is not enough: `run_stdio_parallel` serves requests from a worker
/// pool, so two `merge_inputs`/`assemble_copy` calls can be in flight at
/// once. A shared name let one request truncate the list while ffmpeg was
/// still reading it (or delete it after the other had opened it), producing
/// an output built from the wrong inputs. Same failure mode, same fix as
/// `artifact::part_path`.
static NEXT_CONCAT_LIST: AtomicU64 = AtomicU64::new(0);

#[path = "transform_assemble.rs"]
mod assemble;
#[path = "transform_manifest.rs"]
mod manifest;
#[path = "transform_merge.rs"]
mod merge;
#[path = "transform_mux.rs"]
mod mux;
#[path = "transform_operations.rs"]
mod operations;

/// Expected multi-input cardinality for [`checked_multi_inputs`].
pub(super) enum ExpectedInputs {
    /// One or more inputs (concat-style operations).
    AtLeastOne,
    /// Exactly N inputs (e.g. video + final audio for the copy-only mux).
    Exactly(usize),
}

/// Shared prologue for multi-input transforms: every input must be a
/// readable file, the output must be set, and the output directory is
/// created. Error strings are byte-identical to the historical per-operation
/// messages, so the JSON wire format is unchanged.
pub(super) fn checked_multi_inputs(
    request: &Request,
    expected: ExpectedInputs,
    missing_inputs_message: &str,
) -> Result<(Vec<String>, String), Box<Response>> {
    let inputs = request.input_paths.clone().unwrap_or_default();
    let count_ok = match expected {
        ExpectedInputs::AtLeastOne => !inputs.is_empty(),
        ExpectedInputs::Exactly(n) => inputs.len() == n,
    };
    if !count_ok {
        return Err(Box::new(failed_response(
            None,
            missing_inputs_message.to_string(),
        )));
    }
    if let Some(path) = inputs.iter().find(|path| !Path::new(path).is_file()) {
        return Err(Box::new(failed_response(
            Some(path.to_string()),
            format!("source file is not readable: {path}"),
        )));
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => {
            return Err(Box::new(failed_response(
                Some(inputs[0].clone()),
                "output_path is required".to_string(),
            )));
        }
    };
    if let Some(parent) = Path::new(&output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(Box::new(failed_response(
                Some(inputs[0].clone()),
                format!("create output directory: {error}"),
            )));
        }
    }
    Ok((inputs, output))
}

/// Shared ffmpeg concat-list writer (merge + assemble): one `file '…'`
/// line per input with ffmpeg concat-demuxer escaping. Returns the list
/// path; the caller removes it after the run.
///
/// The file is created with `create_new`, so an existing name is never
/// truncated: a collision retries with the next counter value instead of
/// clobbering a list another in-flight request is reading.
pub(super) fn write_concat_list(inputs: &[String], tag: &str) -> Result<PathBuf, String> {
    let content = inputs
        .iter()
        .map(|path| {
            format!(
                "file '{}'",
                path.replace('\\', "\\\\").replace('\'', "'\\''")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    for _ in 0..32 {
        let unique = NEXT_CONCAT_LIST.fetch_add(1, Ordering::Relaxed);
        let list_path = std::env::temp_dir().join(format!(
            "pipelinegen_{tag}_{}_{unique}.txt",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&list_path)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())
                    .map_err(|error| format!("write concat list: {error}"))?;
                return Ok(list_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("write concat list: {error}")),
        }
    }
    Err("write concat list: could not allocate a unique temporary name".to_string())
}

/// Shared epilogue for ffmpeg transforms: publish the part file on success,
/// map failures to the historical `"{operation} failed…"` messages. The
/// part file is always removed on failure. `metadata` carries operation
/// extras (e.g. the mux copy-eligibility report); `None` keeps the plain
/// shape. Error strings are byte-identical to the historical per-operation
/// messages, so the JSON wire format is unchanged.
pub(super) fn finish_run(
    operation: &str,
    source: &str,
    part: &str,
    output: &str,
    result: std::io::Result<crate::process::ProcessOutput>,
    metadata: Option<MediaMetadata>,
) -> Response {
    match result {
        Ok(result) if result.status.success() => match publish_output(part, output) {
            Ok(()) => Response {
                ok: true,
                operation: operation.to_string(),
                source_path: Some(source.to_string()),
                items: Vec::new(),
                metadata,
                preflight: None,
                error: None,
            },
            Err(error) => failed_response(Some(source.to_string()), error),
        },
        Ok(result) => {
            let _ = fs::remove_file(part);
            failed_response(
                Some(source.to_string()),
                format!(
                    "{operation} failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(part);
            failed_response(
                Some(source.to_string()),
                format!("{operation} failed to start: {error}"),
            )
        }
    }
}

pub(crate) fn execute(request: Request, operation: Operation) -> Response {
    match operation {
        Operation::MergeInputs => merge::execute(request),
        Operation::MuxAudioCopy => mux::execute(request),
        Operation::AssembleCopy => assemble::execute(request),
        Operation::ProjectOverlayManifest => manifest::execute_projection(request),
        Operation::ExtractTimelineWindow => manifest::execute_window(request),
        Operation::Normalize
        | Operation::CutCopy
        | Operation::CutAndNormalize
        | Operation::ExtractFrame
        | Operation::GenerateProxy
        | Operation::GenerateStoryboard
        | Operation::RemuxHls
        | Operation::RemoveSilence
        | Operation::Trim => operations::execute(request, operation),
        other => failed_response(
            request.source_path.clone(),
            format!("not a transform operation: {}", other.as_str()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::write_concat_list;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn concat_lists_are_unique_and_keep_their_own_inputs() {
        let inputs: Vec<Vec<String>> = (0..16)
            .map(|index| vec![format!("/tmp/input-{index}.mp4")])
            .collect();
        let written = std::thread::scope(|scope| {
            let handles: Vec<_> = inputs
                .iter()
                .map(|batch| scope.spawn(move || write_concat_list(batch, "concat")))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("concat writer"))
                .collect::<Result<Vec<_>, _>>()
                .expect("every list must be written")
        });
        let paths: HashSet<_> = written.iter().collect();
        assert_eq!(paths.len(), inputs.len(), "paths must not collide");
        for (path, batch) in written.iter().zip(&inputs) {
            let content = fs::read_to_string(path).expect("concat list");
            assert_eq!(content, format!("file '{}'", batch[0]));
            fs::remove_file(path).expect("cleanup");
        }
    }

    #[test]
    fn concat_list_escapes_backslashes_and_quotes() {
        let path =
            write_concat_list(&["/tmp/a'b\\c.mp4".to_string()], "concat").expect("concat list");
        let content = fs::read_to_string(&path).expect("concat list");
        assert_eq!(content, "file '/tmp/a'\\''b\\\\c.mp4'");
        fs::remove_file(path).expect("cleanup");
    }
}
