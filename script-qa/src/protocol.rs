use serde_json::json;

use crate::gate::evaluate;
use crate::model::GateRequest;

/// Handle one `quality_gate` NDJSON line. `ok` reports protocol success;
/// the gate verdict lives in `quality.passed` and `reasons`.
pub fn handle_line(line: &str) -> String {
    let request: GateRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return json!({
                "ok": false,
                "error": format!("invalid quality_gate request: {error}"),
            })
            .to_string();
        }
    };

    match evaluate(&request) {
        Some(evaluation) => json!({
            "ok": true,
            "quality": evaluation.quality,
            "reasons": evaluation.reasons,
        })
        .to_string(),
        None => json!({
            "ok": true,
            "quality": null,
            "reasons": [],
        })
        .to_string(),
    }
}
