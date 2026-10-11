#[cfg(test)]
use crate::error::MuscleError;
#[cfg(test)]
use crate::protocol;

// Test-only: production render_stock is canonical-plan-only (render_stock.rs
// routes exclusively to render_stock_canonical); these validators exist so the
// dispatcher contract tests can pin the Go↔Rust transition/effect selection
// IDs. Compiled out of production builds instead of silenced with
// #[allow(dead_code)].
#[cfg(test)]
pub(crate) fn validate_resolved_render_plan(
    input_count: usize,
    no_transitions: bool,
    transitions: &[protocol::RenderTransition],
    no_effects: bool,
    effects: &[protocol::RenderEffectPath],
) -> Result<(), MuscleError> {
    if !no_transitions {
        if transitions.is_empty() {
            return Err(MuscleError::generic(
                "unresolved render plan: transitions are required",
            ));
        }
        for transition in transitions {
            if transition.clip_index >= input_count
                || !matches!(transition.segment.as_str(), "start" | "end")
                || !supported_transition(&transition.id)
            {
                return Err(MuscleError::generic(format!(
                    "invalid resolved transition: {}",
                    transition.id
                )));
            }
        }
    }
    if !no_effects {
        if effects.is_empty() {
            return Err(MuscleError::generic(
                "unresolved render plan: effect paths are required",
            ));
        }
        for effect in effects {
            if effect.clip_index >= input_count || effect.path.trim().is_empty() {
                return Err(MuscleError::generic(format!(
                    "invalid resolved effect path: {}",
                    effect.path
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn supported_transition(name: &str) -> bool {
    matches!(
        name,
        "fadeblack"
            | "fadewhite"
            | "flash"
            | "blur"
            | "gray"
            | "colorred"
            | "colorblue"
            | "colorgreen"
            | "coloryellow"
            | "colorpurple"
            | "colororange"
            | "colorpink"
            | "negate"
            | "vignette"
            | "fastblur"
    )
}
