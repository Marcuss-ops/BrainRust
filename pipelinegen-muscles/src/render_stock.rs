use crate::protocol::{Request, Response};

#[path = "render_stock_canonical.rs"]
mod canonical;
#[path = "render_stock_effects.rs"]
mod effects;

// Runtime stock rendering is canonical-plan-only. Legacy render implementations
// have been physically removed; requests without render_plan fail closed.
pub(crate) fn execute(request: Request) -> Response {
    if request.render_plan.is_none() {
        return crate::artifact::failed_response(
            None,
            "render_stock requires a canonical render_plan".to_string(),
        );
    }
    canonical::render_stock_canonical(request)
}

pub(crate) use effects::reject_unresolved_selection;

// These two are consumed by the dispatcher tests through the render_stock
// path; production code reaches them through the effects module directly.
#[cfg(test)]
pub(crate) use effects::{supported_transition, validate_resolved_render_plan};
