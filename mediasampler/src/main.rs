use mediasampler::{
    sample_scene, sample_scene_group, BoundAssets, Candidate, SampleOptions, SampleResult, Scene,
};
use ndjson_stdio::{check_envelope, ErrorCode, WorkerError};
use serde::{Deserialize, Serialize};

/// One ledger entry binding an asset to the scene that won it. The worker
/// itself stays stateless across stdin lines: the persistent-process pool on
/// the Go side multiplexes unrelated scenes/videos onto the same worker, so
/// in-process cross-line state would leak bindings between unrelated runs.
/// Cross-scene reuse is therefore supplied per request by the caller (which
/// owns the composition), mirroring the `BoundAssets` ledger the library
/// threads through `sample_scene`.
#[derive(Debug, Deserialize)]
struct BoundEntry {
    asset_id: String,
    scene_id: String,
    /// The label the asset won under (optional); it powers the V2 diversity
    /// overlap for pre-bound assets supplied by the Go caller. An unlabeled
    /// entry binds with no diversity evidence rather than failing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SampleRequest {
    // Envelope tolerated for forward compatibility with the Go adapter,
    // which sends version/operation alongside the selection payload.
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    operation: Option<String>,
    scene: Scene,
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    allow_reuse: bool,
    /// Passthrough for `SampleOptions.images_per_scene` (0 disables).
    #[serde(default)]
    images_per_scene: usize,
    /// Per-request cross-scene ledger (see `BoundEntry`). Empty (the
    /// default) preserves the historical single-scene behavior.
    #[serde(default)]
    bound_assets: Vec<BoundEntry>,
    /// `select_group` size (MultiImage 2.0, 2–5 winners). Ignored for
    /// `select`.
    #[serde(default)]
    group_size: usize,
    /// V2 diversity stage weight. Absent keeps the default; values outside
    /// `[0.0, 0.1]` fail closed (the stage is bounded by its blend weight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diversity_weight: Option<f32>,
    /// Data-driven keyword tables. Absent (the default) keeps the curated
    /// V1 tables so a Go-side domain that does not send them stays on the
    /// certified behaviour; supplied lists REPLACE the tables wholesale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mismatched_subjects: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compatible_keywords: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action_verbs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_keywords: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct SampleResponse {
    results: Vec<SampleResult>,
    winner_id: Option<String>,
    /// `select_group` members in rank order (absent for `select`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Applies the optional data-driven table overrides from the request.
/// Each override REPLACES the default table wholesale (never merges), so a
/// domain list stays auditable end-to-end. `diversity_weight` outside
/// `[0.0, 0.1]` fails closed.
fn apply_table_overrides(
    mut options: SampleOptions,
    request: &SampleRequest,
) -> Result<SampleOptions, WorkerError> {
    if let Some(weight) = request.diversity_weight {
        if !(0.0..=0.1).contains(&weight) {
            return Err(WorkerError::new(
                ErrorCode::Execution,
                format!("mediasampler diversity_weight {weight} outside [0.0, 0.1]"),
            ));
        }
        options.diversity_weight = weight;
    }
    if let Some(list) = request.mismatched_subjects.clone() {
        options.mismatched_subjects = list;
    }
    if let Some(list) = request.compatible_keywords.clone() {
        options.compatible_keywords = list;
    }
    if let Some(list) = request.action_verbs.clone() {
        options.action_verbs = list;
    }
    if let Some(list) = request.context_keywords.clone() {
        options.context_keywords = list;
    }
    Ok(options)
}

fn error_response(error: &WorkerError) -> String {
    error.log();
    serde_json::json!({
        "results": [],
        "winner_id": null,
        "error": error.message,
        "error_code": error.code,
    })
    .to_string()
}

fn main() {
    if let Err(error) = ndjson_stdio::run_stdio(process_line) {
        eprintln!("mediasampler read: {error}");
        std::process::exit(1);
    }
}

/// Maps one request line to one response line. Never exits on a malformed
/// line: the Go side drives this binary as a persistent worker, so killing
/// the process would drop every queued request behind it.
fn process_line(line: &str) -> String {
    let request: SampleRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(&WorkerError::new(
                ErrorCode::Decode,
                format!("mediasampler decode: {error}"),
            ));
        }
    };
    // Fail closed on unknown envelope values instead of silently ignoring
    // them: a contract drift between Go and Rust must surface as an error,
    // not as a mis-sampled scene. `select_group` is the second active
    // operation; anything else stays the plain `select`.
    let operation = request.operation.as_deref().unwrap_or("select");
    let group_operation = operation == "select_group";
    if let Err(error) = check_envelope(
        "mediasampler",
        request.version.as_deref(),
        "mediasampler.v1",
        request.operation.as_deref(),
        if group_operation {
            "select_group"
        } else {
            "select"
        },
    ) {
        return error_response(&error);
    }
    // Fail closed on out-of-range soft signals before any scoring happens:
    // an embedding outside [0, 1] is a contract drift from the caller, not
    // something to silently clamp into a wrong winner.
    for candidate in &request.candidates {
        if let Some(value) = candidate.embedding_similarity {
            if !(0.0..=1.0).contains(&value) {
                return error_response(&WorkerError::new(
                    ErrorCode::Execution,
                    format!(
                        "mediasampler candidate {} embedding_similarity {value} outside [0, 1]",
                        candidate.id
                    ),
                ));
            }
        }
    }
    let mut bound = BoundAssets::new();
    for entry in &request.bound_assets {
        bound.bind_label(
            &entry.asset_id,
            &entry.scene_id,
            entry.label.as_deref().unwrap_or(""),
        );
    }
    let options = SampleOptions {
        allow_reuse: request.allow_reuse,
        images_per_scene: request.images_per_scene,
        ..SampleOptions::default()
    };
    let options = match apply_table_overrides(options, &request) {
        Ok(options) => options,
        Err(error) => return error_response(&error),
    };
    if group_operation {
        return match sample_scene_group(
            &request.scene,
            &request.candidates,
            &options,
            &mut bound,
            request.group_size,
        ) {
            Ok((results, group)) => match serde_json::to_string(&SampleResponse {
                results,
                winner_id: group.first().cloned(),
                group_ids: Some(group),
                error: None,
            }) {
                Ok(line) => line,
                Err(error) => error_response(&WorkerError::new(
                    ErrorCode::Encode,
                    format!("mediasampler encode: {error}"),
                )),
            },
            Err(message) => error_response(&WorkerError::new(
                ErrorCode::Execution,
                format!("mediasampler select_group: {message}"),
            )),
        };
    }
    let (results, winner_id) =
        sample_scene(&request.scene, &request.candidates, &options, &mut bound);
    match serde_json::to_string(&SampleResponse {
        results,
        winner_id,
        group_ids: None,
        error: None,
    }) {
        Ok(line) => line,
        Err(error) => error_response(&WorkerError::new(
            ErrorCode::Encode,
            format!("mediasampler encode: {error}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> serde_json::Value {
        serde_json::json!({
            "id": "scene-group-01",
            "subject": "greek salad",
            "terms": ["feta", "tomatoes", "olives"],
        })
    }

    fn candidates() -> serde_json::Value {
        serde_json::json!([
            {"id": "artlist-salad-hero", "label": "greek salad close up", "generic_similarity": 0.9},
            {"id": "artlist-feta-block", "label": "feta cheese block", "generic_similarity": 0.85},
            {"id": "artlist-tomato-bed", "label": "tomatoes on the counter", "generic_similarity": 0.8},
            {"id": "artlist-olives-jar", "label": "olive jar", "generic_similarity": 0.75},
            {"id": "artlist-kitchen-table", "label": "mediterranean kitchen table", "generic_similarity": 0.6},
            {"id": "artlist-boxing", "label": "woman boxing", "generic_similarity": 0.95},
        ])
    }

    fn select_group_request(operation: &str, group_size: serde_json::Value) -> String {
        serde_json::json!({
            "version": "mediasampler.v1",
            "operation": operation,
            "scene": scene(),
            "candidates": candidates(),
            "group_size": group_size,
        })
        .to_string()
    }

    fn parse(response: &str) -> serde_json::Value {
        serde_json::from_str(response).expect("response must be a JSON object")
    }

    #[test]
    fn select_group_returns_ordered_group_ids_of_requested_size() {
        for (size, expected) in [
            (2usize, vec!["artlist-salad-hero", "artlist-feta-block"]),
            (
                3usize,
                vec![
                    "artlist-salad-hero",
                    "artlist-feta-block",
                    "artlist-tomato-bed",
                ],
            ),
            (
                4usize,
                vec![
                    "artlist-salad-hero",
                    "artlist-feta-block",
                    "artlist-tomato-bed",
                    "artlist-olives-jar",
                ],
            ),
            (
                5usize,
                vec![
                    "artlist-salad-hero",
                    "artlist-feta-block",
                    "artlist-tomato-bed",
                    "artlist-olives-jar",
                    "artlist-kitchen-table",
                ],
            ),
        ] {
            let response = process_line(&select_group_request("select_group", size.into()));
            let value = parse(&response);
            assert!(value["error"].is_null(), "size={size}: {response}");
            let group: Vec<String> = value["group_ids"]
                .as_array()
                .expect("group_ids must be present for select_group")
                .iter()
                .map(|id| id.as_str().expect("id").to_string())
                .collect();
            assert_eq!(group.len(), size, "size={size}: {group:?}",);
            // The boxing mismatch must never enter any group.
            assert!(
                !group.iter().any(|id| id.contains("boxing")),
                "size={size}: mismatched asset selected: {group:?}"
            );
            let distinct: std::collections::HashSet<&String> = group.iter().collect();
            assert_eq!(distinct.len(), size, "assets must be distinct: {group:?}");
            for (i, id) in group.iter().enumerate() {
                assert_eq!(id, &expected[i], "size={size}: rank order {group:?}");
            }
            assert_eq!(value["winner_id"], group[0]);
        }
    }

    #[test]
    fn select_group_passes_bound_assets_from_the_caller() {
        let request = serde_json::json!({
            "version": "mediasampler.v1",
            "operation": "select_group",
            "scene": scene(),
            "candidates": candidates(),
            "group_size": 3,
            "bound_assets": [
                {"asset_id": "artlist-salad-hero", "scene_id": "mediterranean-00-opening"},
            ],
        });
        let value = parse(&process_line(&request.to_string()));
        assert!(value["error"].is_null(), "response: {value}");
        let group: Vec<String> = value["group_ids"]
            .as_array()
            .expect("group_ids")
            .iter()
            .map(|id| id.as_str().expect("id").to_string())
            .collect();
        assert!(
            !group.iter().any(|id| id == "artlist-salad-hero"),
            "pre-bound asset must not reappear: {group:?}"
        );
        // The rejected rebinding shows up in the results as already_bound.
        let rejected = value["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["candidate_id"] == "artlist-salad-hero")
            .expect("pre-bound candidate reported");
        assert_eq!(rejected["rejection"], "already_bound");
    }

    #[test]
    fn select_group_rejects_sizes_outside_two_to_five_as_worker_errors() {
        for size in [0usize, 1, 6, 99] {
            let value = parse(&process_line(&select_group_request(
                "select_group",
                size.into(),
            )));
            assert_eq!(value["error_code"], "execution", "size={size}: {value}");
            assert!(
                value["error"]
                    .as_str()
                    .is_some_and(|message| message.contains("2..=5")),
                "size={size}: error must name the valid range: {value}"
            );
        }
    }

    #[test]
    fn select_keeps_its_single_winner_contract() {
        let value = parse(&process_line(&select_group_request("select", 3.into())));
        assert!(value["error"].is_null(), "response: {value}");
        assert!(
            value["group_ids"].is_null(),
            "select must not emit group_ids: {value}"
        );
        assert_eq!(value["winner_id"], "artlist-salad-hero");
    }

    #[test]
    fn data_driven_tables_reject_the_custom_domain_mismatch() {
        // With the default culinary tables, "goalkeeper diving save" is not
        // in the curated mismatched list (boxing/gym/…), so it WOULD score.
        // The request replaces the table; the worker honours it.
        let request = serde_json::json!({
            "version": "mediasampler.v1",
            "scene": {"id": "s-01", "subject": "glacier climb", "terms": ["summit"]},
            "candidates": [
                {"id": "a-goalie", "label": "goalkeeper diving save", "generic_similarity": 0.95},
                {"id": "b-summit", "label": "summit ridge at dawn", "generic_similarity": 0.85},
            ],
            "mismatched_subjects": ["goalkeeper"],
        });
        let value = parse(&process_line(&request.to_string()));
        assert!(value["error"].is_null(), "response: {value}");
        assert_eq!(value["winner_id"], "b-summit");
        let rejected = value["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["candidate_id"] == "a-goalie")
            .expect("mismatched candidate reported");
        assert_eq!(rejected["rejection"], "subject_mismatch");
    }

    #[test]
    fn out_of_range_signal_values_fail_closed() {
        let payload = |field: &str, value: serde_json::Value| {
            serde_json::json!({
                "version": "mediasampler.v1",
                "scene": {"id": "s-01", "subject": "greek salad", "terms": []},
                "candidates": [
                    {"id": "a", "label": "salad", "generic_similarity": 0.5, field: value},
                ],
            })
        };
        let embedding = payload("embedding_similarity", 1.5.into()).to_string();
        let value = parse(&process_line(&embedding));
        assert_eq!(value["error_code"], "execution", "embedding: {value}");
        assert!(value["error"].as_str().unwrap().contains("[0, 1]"));

        let request = serde_json::json!({
            "version": "mediasampler.v1",
            "scene": {"id": "s-01", "subject": "greek salad", "terms": []},
            "candidates": [],
            "diversity_weight": 0.5,
        });
        let value = parse(&process_line(&request.to_string()));
        assert_eq!(value["error_code"], "execution", "weight: {value}");
        assert!(value["error"].as_str().unwrap().contains("[0.0, 0.1]"));
    }

    #[test]
    fn pre_bound_entries_carry_their_label_for_the_diversity_stage() {
        let request = serde_json::json!({
            "version": "mediasampler.v1",
            "operation": "select_group",
            "scene": scene(),
            "candidates": [
                {"id": "artlist-close-up", "label": "salad close up", "generic_similarity": 0.7},
                {"id": "artlist-plate", "label": "salad served on plate", "generic_similarity": 0.7},
            ],
            "group_size": 2,
            "bound_assets": [
                {"asset_id": "artlist-old-salad", "scene_id": "scene-00", "label": "greek salad close up"},
            ],
        });
        let value = parse(&process_line(&request.to_string()));
        assert!(value["error"].is_null(), "response: {value}");
        // The label-overlapping candidate (salad close up vs. bound "{salad}
        // close up") must be demoted; the distinct one leads the group.
        assert_eq!(
            value["group_ids"][0], "artlist-plate",
            "diversity evidence must reorder identical-base candidates: {value}"
        );
        assert_eq!(value["winner_id"], "artlist-plate");
    }
}
