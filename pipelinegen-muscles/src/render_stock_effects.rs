use crate::protocol::{self, Request};

pub(crate) fn reject_unresolved_selection(request: &Request) -> Option<String> {
    if request.transition_every.is_some()
        || request.effects_dir.is_some()
        || request.effect_every.is_some()
        || request.effect_index_hint.is_some()
    {
        return Some(
            "unresolved transition/effect selection is not supported; Go must send explicit IDs and paths"
                .to_string(),
        );
    }
    None
}

pub(crate) fn validate_resolved_render_plan(
    input_count: usize,
    no_transitions: bool,
    transitions: &[protocol::RenderTransition],
    no_effects: bool,
    effects: &[protocol::RenderEffectPath],
) -> Result<(), String> {
    if !no_transitions {
        if transitions.is_empty() {
            return Err("unresolved render plan: transitions are required".to_string());
        }
        for transition in transitions {
            if transition.clip_index >= input_count
                || !matches!(transition.segment.as_str(), "start" | "end")
                || !supported_transition(&transition.id)
            {
                return Err(format!("invalid resolved transition: {}", transition.id));
            }
        }
    }
    if !no_effects {
        if effects.is_empty() {
            return Err("unresolved render plan: effect paths are required".to_string());
        }
        for effect in effects {
            if effect.clip_index >= input_count || effect.path.trim().is_empty() {
                return Err(format!("invalid resolved effect path: {}", effect.path));
            }
        }
    }
    Ok(())
}

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

pub(crate) fn transition_filter(name: &str, duration: i32, start: bool) -> String {
    let at = if start { 0.0 } else { duration as f64 - 0.5 };
    match name {
        "fadeblack" => format!(
            "fade=t={}:st={:.6}:d=0.5",
            if start { "in" } else { "out" },
            at
        ),
        "fadewhite" => format!(
            "fade=t={}:st={:.6}:d=0.5:color=white",
            if start { "in" } else { "out" },
            at
        ),
        "flash" => format!(
            "fade=t={}:st={:.6}:d=0.2:color=white",
            if start { "in" } else { "out" },
            if start { 0.0 } else { duration as f64 - 0.2 }
        ),
        "blur" => format!(
            "boxblur=15:enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "gray" => format!(
            "fade=t={}:st={:.6}:d=0.5:color=gray",
            if start { "in" } else { "out" },
            at
        ),
        "negate" => format!(
            "negate=enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "vignette" => format!(
            "vignette=enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "fastblur" => format!(
            "boxblur=30:enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        color => {
            let color = color.strip_prefix("color").unwrap_or("black");
            format!(
                "fade=t={}:st={:.6}:d=0.5:color={}",
                if start { "in" } else { "out" },
                at,
                color
            )
        }
    }
}
