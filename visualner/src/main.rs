use ndjson_stdio::{check_envelope, ErrorCode, WorkerError};
use serde::{Deserialize, Serialize};
use visualner::{
    analyze, extract, supported_spellout_language_count, ExtractOptions, VisualEntity,
};

#[derive(Debug, Deserialize)]
struct ExtractRequest {
    // Envelope tolerated for forward compatibility with the Go adapter,
    // which sends version/operation/segment_id/text_hash alongside the
    // extraction payload. None of them change extraction semantics.
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    segment_id: Option<String>,
    #[serde(default)]
    text_hash: Option<String>,
    source_text: String,
    language: String,
    #[serde(default)]
    entity_count: usize,
    /// Compatibility alias for `entity_count` sent by older runners.
    /// Used only when `entity_count` is zero.
    #[serde(default)]
    limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_requires_language() {
        let missing =
            serde_json::from_str::<ExtractRequest>(r#"{"source_text":"twenty-five percent"}"#);
        assert!(
            missing.is_err(),
            "missing language must be rejected at the CLI boundary"
        );

        let present = serde_json::from_str::<ExtractRequest>(
            r#"{"source_text":"twenty-five percent","language":"en"}"#,
        );
        assert_eq!(present.unwrap().language, "en");
    }

    #[test]
    fn limit_falls_back_to_entity_count() {
        let req = serde_json::from_str::<ExtractRequest>(
            r#"{"source_text":"feta cheese","language":"en","limit":5}"#,
        )
        .unwrap();
        assert_eq!(effective_entity_count(&req), 5);
    }

    #[test]
    fn error_response_carries_code_and_empty_entities() {
        let error = WorkerError::new(ErrorCode::Decode, "visualner decode: boom");
        let line = error_response(&error);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["entities"], serde_json::json!([]));
        assert_eq!(value["error"], "visualner decode: boom");
        assert_eq!(value["error_code"], "decode");
    }
}

fn effective_entity_count(request: &ExtractRequest) -> usize {
    if request.entity_count > 0 {
        request.entity_count
    } else {
        request.limit
    }
}

#[derive(Debug, Serialize)]
struct ExtractResponse {
    entities: Vec<VisualEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn error_response(error: &WorkerError) -> String {
    error.log();
    serde_json::json!({
        "entities": [],
        "error": error.message,
        "error_code": error.code,
    })
    .to_string()
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--locale-count") {
        println!("{}", supported_spellout_language_count());
        return;
    }
    if let Err(error) = ndjson_stdio::run_stdio(process_line) {
        eprintln!("visualner read: {error}");
        std::process::exit(1);
    }
}

/// Maps one request line to one response line. Never exits on a malformed
/// line: the Go side drives this binary as a persistent worker, so killing
/// the process would drop every queued request behind it.
fn process_line(line: &str) -> String {
    let request: ExtractRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(&WorkerError::new(
                ErrorCode::Decode,
                format!("visualner decode: {error}"),
            ));
        }
    };
    // Fail closed on unknown envelope values instead of silently ignoring
    // them: a contract drift between Go and Rust must surface as an error,
    // not as mis-extracted entities.
    let _ = request.segment_id.as_deref();
    let _ = request.text_hash.as_deref();
    let analyze_op = request.operation.as_deref() == Some("analyze");
    let expected_operation = if analyze_op { "analyze" } else { "extract" };
    if let Err(error) = check_envelope(
        "visualner",
        request.version.as_deref(),
        "visualner.v1",
        request.operation.as_deref(),
        expected_operation,
    ) {
        return error_response(&error);
    }
    let entity_count = effective_entity_count(&request);
    if analyze_op {
        return match analyze(
            &request.source_text,
            &ExtractOptions {
                language: request.language,
                entity_count,
            },
        ) {
            Ok(analysis) => serde_json::to_string(&analysis).unwrap_or_else(|error| {
                error_response(&WorkerError::new(
                    ErrorCode::Encode,
                    format!("visualner encode: {error}"),
                ))
            }),
            Err(error) => error_response(&WorkerError::new(
                ErrorCode::Execution,
                format!("visualner analyze: {error}"),
            )),
        };
    }
    let entities = match extract(
        &request.source_text,
        &ExtractOptions {
            language: request.language,
            entity_count,
        },
    ) {
        Ok(entities) => entities,
        Err(error) => {
            return error_response(&WorkerError::new(
                ErrorCode::Execution,
                format!("visualner extract: {error}"),
            ));
        }
    };
    let response = ExtractResponse {
        entities,
        error: None,
    };
    match serde_json::to_string(&response) {
        Ok(line) => line,
        Err(error) => error_response(&WorkerError::new(
            ErrorCode::Encode,
            format!("visualner encode: {error}"),
        )),
    }
}
