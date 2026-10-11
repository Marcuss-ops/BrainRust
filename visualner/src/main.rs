use serde::{Deserialize, Serialize};
use visualner::{extract, supported_spellout_language_count, ExtractOptions, VisualEntity};

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

fn error_response(message: String) -> String {
    serde_json::json!({"entities": [], "error": message}).to_string()
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
            let message = format!("visualner decode: {error}");
            eprintln!("{message}");
            return error_response(message);
        }
    };
    // Fail closed on unknown envelope values instead of silently
    // ignoring them: a contract drift between Go and Rust must surface
    // as an error, not as mis-extracted entities. The `_` bindings keep
    // the envelope fields acknowledged without changing semantics.
    let _ = request.segment_id.as_deref();
    let _ = request.text_hash.as_deref();
    if let Some(version) = request.version.as_deref() {
        if version != "visualner.v1" {
            let message = format!("visualner: unsupported version: {version}");
            eprintln!("{message}");
            return error_response(message);
        }
    }
    if let Some(operation) = request.operation.as_deref() {
        if operation != "extract" {
            let message = format!("visualner: unsupported operation: {operation}");
            eprintln!("{message}");
            return error_response(message);
        }
    }
    let entity_count = effective_entity_count(&request);
    let entities = match extract(
        &request.source_text,
        &ExtractOptions {
            language: request.language,
            entity_count,
        },
    ) {
        Ok(entities) => entities,
        Err(error) => {
            let message = format!("visualner extract: {error}");
            eprintln!("{message}");
            return error_response(message);
        }
    };
    let response = ExtractResponse {
        entities,
        error: None,
    };
    serde_json::to_string(&response).expect("visualner encode")
}
