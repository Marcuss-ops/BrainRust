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

pub(crate) use effects::{
    reject_unresolved_selection, supported_transition, transition_filter,
    validate_resolved_render_plan,
};
