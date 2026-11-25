use super::*;

pub(super) fn validate_asset_ref(
    label: &str,
    asset: &AssetRefV2,
) -> Result<(), EditorialPlanError> {
    if asset.asset_id.trim().is_empty() {
        return Err(EditorialPlanError::invalid(label, "asset_id is required"));
    }
    if !asset.sha256.trim().is_empty() && !is_lower_hex_64(&asset.sha256) {
        return Err(EditorialPlanError::invalid(
            label,
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    reject_placeholder_url(label, &asset.url)
}

pub(super) fn validate_image_candidate(
    label: &str,
    candidate: &ImageCandidateV2,
) -> Result<(), EditorialPlanError> {
    if candidate.asset_id.trim().is_empty() && candidate.url.trim().is_empty() {
        return Err(EditorialPlanError::invalid(
            label,
            "image candidate needs an asset_id or a url",
        ));
    }
    if !candidate.sha256.trim().is_empty() && !is_lower_hex_64(&candidate.sha256) {
        return Err(EditorialPlanError::invalid(
            label,
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    if candidate.duration_us < 0 || candidate.start_us < 0 {
        return Err(EditorialPlanError::invalid(label, "negative image window"));
    }
    reject_placeholder_url(label, &candidate.url)
}

/// A semantic catalog reference is a database identity until its bytes are
/// materialized. It can never be a render asset, so it is a contract error
/// here instead of a silently skipped item later.
pub(super) fn reject_placeholder_url(label: &str, url: &str) -> Result<(), EditorialPlanError> {
    let url = url.trim();
    if url.starts_with("assets/semantic/") || url.starts_with("semantic/") {
        return Err(EditorialPlanError::invalid(
            label,
            format!("unmaterialized semantic placeholder URL {url:?}"),
        ));
    }
    Ok(())
}

/// Closed set of grounded recipes. The three layout-only recipes stay valid at
/// the contract boundary but the planner does not lower them: TICKET-48 measured
/// `relationship_graph` 3, `lifespan_timeline` 1 and `choropleth_value_map` 0 in
/// 1,173 archived scenes and decided to keep them validated-but-inert.
pub(super) fn is_supported_visual_recipe(recipe: &str) -> bool {
    matches!(
        recipe,
        "relationship_graph"
            | "role_lower_third"
            | "route_map"
            | "choropleth_value_map"
            | "lifespan_timeline"
            | "counter_odometer"
            | "quote_attribution"
            | "source_highlight"
            | "relative_time_jump"
            | "cause_effect_card"
            | "process_stepper"
    )
}

pub(super) fn validate_visual_recipe(
    label: &str,
    payload_label: &str,
    recipe: &str,
    payload: Option<&serde_json::Value>,
) -> Result<(), EditorialPlanError> {
    let recipe = recipe.trim();
    if !recipe.is_empty() && !is_supported_visual_recipe(recipe) {
        return Err(EditorialPlanError::invalid(
            label,
            "unsupported grounded visual recipe",
        ));
    }
    if payload.is_some_and(|payload| !payload.is_object()) {
        return Err(EditorialPlanError::invalid(
            payload_label,
            "visual payload must be an object",
        ));
    }
    Ok(())
}

pub(super) fn validate_visual_recipe_list(
    label: &str,
    recipes: &[VisualRecipeSpecV2],
) -> Result<(), EditorialPlanError> {
    // A scene states facts, not a film strip: eight grounded recipes is already
    // beyond any current producer and bounds the lowering loop.
    if recipes.len() > 8 {
        return Err(EditorialPlanError::invalid(
            format!("{label}.visual_recipes"),
            "too many visual recipes for one scene",
        ));
    }
    for (index, spec) in recipes.iter().enumerate() {
        if spec.recipe.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("{label}.visual_recipes[{index}].recipe"),
                "recipe is required",
            ));
        }
        validate_visual_recipe(
            &format!("{label}.visual_recipes[{index}].recipe"),
            &format!("{label}.visual_recipes[{index}].payload"),
            &spec.recipe,
            spec.payload.as_ref(),
        )?;
    }
    Ok(())
}
