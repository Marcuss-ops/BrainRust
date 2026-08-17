use crate::protocol::{Request, Response};

#[path = "render_stock_canonical.rs"]
mod canonical;
#[path = "render_stock_effects.rs"]
mod effects;
#[path = "render_stock_legacy.rs"]
mod legacy;

pub(crate) fn execute(request: Request) -> Response {
    if request.render_plan.is_some() {
        return canonical::render_stock_canonical(request);
    }
    legacy::render_stock(request)
}

pub(crate) use effects::reject_unresolved_selection;

// These two are consumed by the dispatcher tests through the render_stock
// path; production code reaches them through the effects module directly.
#[cfg(test)]
pub(crate) use effects::{supported_transition, validate_resolved_render_plan};
