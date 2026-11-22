//! Lowering of `visual_recipe` / `visual_payload` onto templates the renderer
//! already certified.
//!
//! A recipe is not a new item kind. Each one either enriches the item that
//! already represents the fact (a number card gains its subject, a quote gains
//! its author, a phrase gains the archival-source entrance) or reorders items
//! that already exist (route pins). That keeps admission, overlap, budgets and
//! the Go seal unaware of the recipe vocabulary, and it makes a recipe that
//! cannot find its grounded item a no-op instead of a duplicate card.
//!
//! Timing always comes from an existing annotation or entity occurrence: the
//! payload carries the fact, never a window, so a recipe cannot invent one.

use super::super::request::SceneAnnotationsV2;
use super::*;

/// Applies every recipe of one scene, in producer priority order, to the items
/// appended for that scene only. `first` is the index where this scene started
/// contributing items. The legacy pair is consumed as the first element; a
/// producer that sends both forms does not apply the same fact twice.
pub(super) fn apply_visual_recipes(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    annotations: &SceneAnnotationsV2,
    request: &EditorialPlanV2Request,
    first: usize,
) {
    let recipe = annotations.visual_recipe.trim();
    let mut skip_first_listed = false;
    if !recipe.is_empty() {
        apply_visual_recipe(
            items,
            scene,
            recipe,
            annotations.visual_payload.as_ref(),
            request,
            first,
        );
        skip_first_listed = annotations.visual_recipes.first().is_some_and(|spec| {
            spec.recipe.trim() == recipe && spec.payload == annotations.visual_payload
        });
    }
    for spec in annotations
        .visual_recipes
        .iter()
        .skip(usize::from(skip_first_listed))
    {
        apply_visual_recipe(
            items,
            scene,
            spec.recipe.trim(),
            spec.payload.as_ref(),
            request,
            first,
        );
    }
}

fn apply_visual_recipe(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    recipe: &str,
    payload: Option<&Value>,
    request: &EditorialPlanV2Request,
    first: usize,
) {
    let Some(payload) = payload else {
        return;
    };
    match recipe {
        "counter_odometer" => apply_counter_odometer(items, scene, payload, request, first),
        "role_lower_third" => {
            append_role_lower_third(items, scene, payload, request);
        }
        "quote_attribution" => apply_quote_attribution(items, payload, first),
        "source_highlight" => apply_source_highlight(items, payload, first),
        "route_map" => apply_route_order(items, payload, first),
        // TICKET-48 decision: these three stay validated-but-inert. Each needs
        // a layout the renderer does not certify, they fired in 3, 1 and 0 of
        // the 1,173 archived scenes, and rejecting them at validation would
        // fail a whole plan over a grounded fact. Go does not forward them.
        "relationship_graph" | "choropleth_value_map" | "lifespan_timeline" => {}
        // Unreachable for a validated request; validation owns the closed set.
        _ => {}
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> &'a str {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

/// Case- and punctuation-insensitive form used only to match two surfaces that
/// the producer and the renderer may have quoted slightly differently.
fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut space = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            out.extend(ch.to_lowercase());
            space = false;
        } else {
            space = true;
        }
    }
    out
}

/// A counter card is the same annotation the numeric lane already built, with
/// the metric subject as a second line. Matching the existing number item is
/// what prevents a second overlay for the same value.
fn apply_counter_odometer(
    items: &mut [PlanItemV2],
    scene: &SceneV2,
    payload: &Value,
    request: &EditorialPlanV2Request,
    first: usize,
) {
    let subject = text(payload, "subject_label");
    if subject.is_empty() || subject.chars().count() > 32 {
        return;
    }
    let Some(value) = payload.get("value").and_then(Value::as_f64) else {
        return;
    };
    let Some(item) = items
        .iter_mut()
        .skip(first)
        .find(|item| item.kind == "number" && numeric_surface_matches(&item.text, value))
    else {
        return;
    };
    // A second recipe for the same value enriches the same card again unless
    // the already-rendered two-line form is left alone.
    if item.text.contains('\n') {
        return;
    }
    let value_line = item.text.trim();
    if value_line.is_empty() || value_line.chars().count() > 18 {
        return;
    }
    item.text = format!("{value_line}\n{subject}");
    item.template_id = "METRIC_STAT_CARD".to_string();
    if let Some(motion) = seeded_preset(
        &request.plan.plan_id,
        &scene.id,
        &item.id,
        "recipe_counter_odometer",
        METRIC_MOTIONS,
    ) {
        item.motion_id = motion;
    }
    let mut params = item
        .params
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    params.insert("position".to_string(), json!("center"));
    params.insert("style".to_string(), json!("stat"));
    params.insert("font_size_px".to_string(), json!(110.0));
    item.params = Some(Value::Object(params));
}

/// The first number in a rendered numeric surface. Currency symbols and
/// thousands separators are ignored; the comparison is exact enough to bind a
/// recipe to its own annotation and loose enough not to care about locale.
fn numeric_surface_matches(surface: &str, value: f64) -> bool {
    let raw: String = surface
        .chars()
        .filter(|ch| ch.is_ascii_digit() || matches!(ch, '.' | ',' | '-'))
        .collect();
    if raw.is_empty() {
        return false;
    }
    // The same digits can be grouped Italian-style (1.234,5) or English-style
    // (1,234.5); try both readings instead of assuming the plan's locale.
    let variants = [
        raw.clone(),
        raw.replace('.', "").replace(',', "."),
        raw.replace(',', ""),
    ];
    variants
        .iter()
        .filter_map(|candidate| candidate.parse::<f64>().ok())
        .any(|parsed| (parsed - value).abs() < 1e-6)
}

/// A role card is the one recipe that owns a new item: it is a lower third,
/// which is a certified template the producer simply never emitted. Timing is
/// the person's own occurrence in the canonical entity timeline.
fn append_role_lower_third(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    payload: &Value,
    request: &EditorialPlanV2Request,
) {
    let person = text(payload, "person_label");
    let role = text(payload, "role_surface");
    if person.is_empty() || role.is_empty() {
        return;
    }
    if person.chars().count() > 48 || role.chars().count() > 48 {
        return;
    }
    if items
        .iter()
        .any(|item| item.kind == "lower_third" && normalize(&item.text) == normalize(person))
    {
        return;
    }
    let organization = text(payload, "organization_label");
    let role_line = if organization.is_empty() {
        role.to_string()
    } else {
        format!("{role}, {organization}")
    };
    let person_id = text(payload, "person_id");
    let Some((start_us, duration_us)) = entity_window(scene, request, person, person_id) else {
        return;
    };
    let mut item = scene_item(
        format!("{}-recipe-lower-third", scene.id),
        scene,
        "lower_third",
        "LOWER_THIRD",
        start_us,
        duration_us,
        format!("{person}\n{role_line}"),
    );
    item.params = Some(json!({
        "position": "lower_third",
        "priority": 0.86,
    }));
    items.push(item);
}

/// Finds the person's spoken window: the canonical entity occurrence first,
/// then a phrase that names them. No match means no card.
fn entity_window(
    scene: &SceneV2,
    request: &EditorialPlanV2Request,
    person: &str,
    person_id: &str,
) -> Option<(i64, i64)> {
    let timeline = request.entity_timeline.as_ref()?;
    let occurrences = &timeline
        .scenes
        .iter()
        .find(|entry| entry.scene_id == scene.id)?
        .entities;
    let occurrence = occurrences.iter().find(|entity| {
        (!person_id.is_empty()
            && (entity.entity_id == person_id || entity.canonical_entity_id == person_id))
            || entity.name.eq_ignore_ascii_case(person)
    })?;
    let start = if occurrence.audio_end_us > occurrence.audio_start_us {
        occurrence.audio_start_us
    } else {
        occurrence
            .timeline_start_us
            .max(scene.timeline_start_us)
            .saturating_add(occurrence.local_start_us)
    };
    let end = if occurrence.audio_end_us > occurrence.audio_start_us {
        occurrence.audio_end_us
    } else {
        start.saturating_add(occurrence.local_end_us - occurrence.local_start_us)
    };
    let duration = (end - start).max(2_500_000);
    Some((start, duration))
}

/// Credits the quote item that already exists. Appending the speaker to a
/// duplicate quote card would show the line twice, so an unmatched quote is
/// left untouched.
fn apply_quote_attribution(items: &mut [PlanItemV2], payload: &Value, first: usize) {
    let quote = normalize(text(payload, "quote_text"));
    let author = text(payload, "author_label");
    if quote.is_empty() || author.is_empty() {
        return;
    }
    let role = text(payload, "role_surface");
    let Some(item) = items
        .iter_mut()
        .skip(first)
        .find(|item| item.kind == "quote" && surfaces_overlap(&normalize(&item.text), &quote))
    else {
        return;
    };
    let credit = if role.is_empty() {
        author.to_string()
    } else {
        format!("{author}, {role}")
    };
    if credit.chars().count() > 60 {
        return;
    }
    // A quote gains one attribution; a repeated recipe must not stack credits.
    if item.text.contains("\n— ") {
        return;
    }
    item.text = format!("{}\n— {credit}", item.text.trim());
}

fn surfaces_overlap(left: &str, right: &str) -> bool {
    !left.is_empty()
        && !right.is_empty()
        && (left == right || left.contains(right) || right.contains(left))
}

/// Pins the certified archival-source entrance on the phrase or quote that
/// names the outlet. No new card: the sentence the viewer already sees is the
/// one that gains the emphasis.
fn apply_source_highlight(items: &mut [PlanItemV2], payload: &Value, first: usize) {
    let source = normalize(text(payload, "source_label"));
    let highlight = normalize(text(payload, "highlight_text"));
    if source.is_empty() {
        return;
    }
    let Some(item) = items.iter_mut().skip(first).find(|item| {
        matches!(item.kind.as_str(), "text_phrase" | "quote")
            && (normalize(&item.text).contains(&source)
                || (!highlight.is_empty() && normalize(&item.text) == highlight))
    }) else {
        return;
    };
    if item.motion_id.trim().is_empty() {
        item.motion_id = "doc_04_archival_source_highlight".to_string();
    }
}

/// Route pins are the scene's existing map items, reordered to the waypoint
/// order the payload carries. `map_groups` later groups them in plan order, so
/// the order here is the pin order on screen.
fn apply_route_order(items: &mut [PlanItemV2], payload: &Value, first: usize) {
    let stops: Vec<String> = payload
        .get("stops")
        .and_then(Value::as_array)
        .map(|stops| {
            stops
                .iter()
                .filter_map(|stop| stop.get("name").and_then(Value::as_str))
                .map(normalize)
                .collect()
        })
        .unwrap_or_default();
    if stops.len() < 2 {
        return;
    }
    let mut positions: Vec<usize> = (first..items.len())
        .filter(|&index| items[index].kind == "map")
        .collect();
    if positions.len() < 2 {
        return;
    }
    let mut ordered: Vec<PlanItemV2> = positions
        .iter()
        .map(|&index| items[index].clone())
        .collect();
    ordered.sort_by_key(|item| {
        let label = item
            .map
            .as_ref()
            .and_then(|map| map.get("label"))
            .and_then(Value::as_str)
            .map(normalize)
            .unwrap_or_default();
        stops
            .iter()
            .position(|stop| *stop == label)
            .unwrap_or(usize::MAX)
    });
    for (position, item) in positions.drain(..).zip(ordered) {
        items[position] = item;
    }
}
