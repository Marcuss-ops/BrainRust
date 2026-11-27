//! TICKET-45 visual recipes for facts the engine already extracts.
//!
//! Every recipe is a `ProductionCandidate` grounded on an existing fact with a
//! verbatim `[start_byte, end_byte)` source span. A missing slot fails closed:
//! no identity, no coordinates, no date or no grounded unit means no recipe.

use std::collections::{BTreeMap, HashMap, HashSet};

use semantic_core::event::EventFrame;
use semantic_core::{ProductionCandidate, SceneInput, SceneManifest};
use serde_json::json;

use super::IdentityState;

const RECIPE_SCHEMA: &str = "visual_recipe.v1";
const CHART_SPAN: &[&str] = &["chart", "delta_comparison", "ranking_bar"];

#[path = "recipes/quotes.rs"]
mod quotes;
use quotes::*;

fn grounded(text: &str, start: usize, end: usize) -> Option<&str> {
    if start >= end {
        return None;
    }
    text.get(start..end)
}

/// Named inputs for one visual-recipe candidate. The previous positional
/// signature had thirteen parameters; the struct keeps each call site
/// self-describing without changing the produced candidate.
struct RecipeCandidate<'a> {
    recipe: &'a str,
    scene_id: &'a str,
    text: &'a str,
    start: usize,
    end: usize,
    title: String,
    suggested_visual: &'a str,
    priority: f32,
    entity_ids: Vec<String>,
    covered_fact_ids: Vec<String>,
    payload: serde_json::Value,
    dedup_key: String,
    rationale: &'a str,
}

fn candidate(input: RecipeCandidate<'_>) -> Option<ProductionCandidate> {
    let RecipeCandidate {
        recipe,
        scene_id,
        text,
        start,
        end,
        title,
        suggested_visual,
        priority,
        entity_ids,
        covered_fact_ids,
        payload,
        dedup_key,
        rationale,
    } = input;
    let evidence = grounded(text, start, end)?.to_string();
    Some(ProductionCandidate {
        id: String::new(),
        recipe: recipe.to_string(),
        rank: 0,
        priority,
        scene_id: scene_id.to_string(),
        scene_ids: vec![scene_id.to_string()],
        entity_ids,
        fact_ids: Vec::new(),
        group_ids: Vec::new(),
        title,
        evidence,
        start_byte: start,
        end_byte: end,
        suggested_visual: suggested_visual.to_string(),
        chart_items: Vec::new(),
        chart_meta: None,
        visual_payload: Some(payload),
        temporal_surfaces: Vec::new(),
        asset_key: dedup_key.clone(),
        queries: Vec::new(),
        requires_media: false,
        dedup_key,
        covered_fact_ids,
        rationale: vec![rationale.to_string()],
    })
}

fn chart_spans(candidates: &[ProductionCandidate]) -> HashMap<String, Vec<(usize, usize)>> {
    let mut out: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for candidate in candidates {
        if !CHART_SPAN.contains(&candidate.recipe.as_str()) {
            continue;
        }
        let spans = out.entry(candidate.scene_id.clone()).or_default();
        if candidate.chart_items.is_empty() {
            spans.push((candidate.start_byte, candidate.end_byte));
        } else {
            spans.extend(
                candidate
                    .chart_items
                    .iter()
                    .map(|item| (item.start_byte, item.end_byte)),
            );
        }
    }
    out
}

fn overlaps(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    spans
        .iter()
        .any(|(left, right)| start < *right && *left < end)
}

fn ties(identities: &BTreeMap<String, IdentityState>, id: &str) -> Option<String> {
    identities
        .get(id)
        .map(|identity| identity.canonical.clone())
}

/// One grouped card per person whose scene has at least two asserted
/// attributes. The payload is a projection of `entity_attribute_lists`; it
/// carries no new extraction and no timing.
fn push_entity_attribute_lists(
    language: &str,
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let ontology = &semantic_core::lexicon::lexicon().attribute_predicates;
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for list in &scene.entity_attribute_lists {
            if list.attributes.len() < 2 || !identities.contains_key(&list.entity_id) {
                continue;
            }
            let Some(evidence) = grounded(text, list.start_byte, list.end_byte) else {
                continue;
            };
            if evidence != list.evidence {
                continue;
            }
            let mut lines = Vec::new();
            let mut attributes = Vec::new();
            let mut covered = Vec::new();
            for attribute in &list.attributes {
                let Some(label) = ontology.label(&attribute.predicate, language) else {
                    continue;
                };
                let mut line = format!("{label} {}", attribute.value);
                if let Some(unit) = attribute.unit_surface.as_deref() {
                    if !unit.is_empty() {
                        line.push(' ');
                        line.push_str(unit);
                    }
                }
                if attribute.qualifier != "exact" {
                    line = format!("~{line}");
                }
                lines.push(line);
                covered.push(attribute.source_fact_id.clone());
                attributes.push(json!({
                    "predicate": attribute.predicate,
                    "value": attribute.value,
                    "normalized_value": attribute.normalized_value,
                    "unit": attribute.unit,
                    "unit_surface": attribute.unit_surface,
                    "qualifier": attribute.qualifier,
                    "status": attribute.status,
                    "start_byte": attribute.start_byte,
                    "end_byte": attribute.end_byte,
                    "sentence_index": attribute.sentence_index,
                    "evidence": attribute.evidence,
                    "source_fact_id": attribute.source_fact_id,
                }));
            }
            if attributes.len() < 2 {
                continue;
            }
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "entity_attribute_list",
                "entity_id": list.entity_id,
                "entity_label": list.label,
                "evidence": evidence,
                "lines": lines,
                "attributes": attributes,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "entity_attribute_list",
                scene_id: scene.scene_id.as_str(),
                text,
                start: list.start_byte,
                end: list.end_byte,
                title: list.label.clone(),
                suggested_visual: "entity_attribute_list",
                priority: 0.84,
                entity_ids: vec![list.entity_id.clone()],
                covered_fact_ids: covered,
                payload,
                dedup_key: format!(
                    "entity_attribute_list:{}:{}",
                    scene.scene_id, list.entity_id
                ),
                rationale: "at least two asserted attributes bound to one grounded identity",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

fn push_relationship_graphs(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let mut edges = Vec::new();
        let mut entity_ids = HashSet::new();
        let mut covered = Vec::new();
        let mut start = usize::MAX;
        let mut end = 0;
        let mut subjects: BTreeMap<String, String> = BTreeMap::new();
        for relation in &scene.person_relations {
            let Some(edge_evidence) = grounded(text, relation.start_byte, relation.end_byte) else {
                continue;
            };
            if relation.status != "asserted" {
                continue;
            }
            let (Some(from), Some(to)) = (
                ties(identities, &relation.from_id),
                ties(identities, &relation.to_id),
            ) else {
                continue;
            };
            subjects.insert(relation.from_id.clone(), from.clone());
            subjects.insert(relation.to_id.clone(), to.clone());
            entity_ids.insert(relation.from_id.clone());
            entity_ids.insert(relation.to_id.clone());
            start = start.min(relation.start_byte);
            end = end.max(relation.end_byte);
            covered.push(format!(
                "relation:{}:{}:{}:{}",
                scene.scene_id, relation.from_id, relation.to_id, relation.kind
            ));
            edges.push(json!({
                "from": relation.from_id,
                "to": relation.to_id,
                "from_label": from,
                "to_label": to,
                "kind": relation.kind,
                "status": relation.status,
                "start_byte": relation.start_byte,
                "end_byte": relation.end_byte,
                "evidence": edge_evidence,
            }));
        }
        if edges.is_empty() {
            continue;
        }
        let payload = json!({
            "schema": RECIPE_SCHEMA,
            "recipe": "relationship_graph",
            "subjects": subjects.iter().map(|(id, label)| json!({"entity_id": id, "label": label})).collect::<Vec<_>>(),
            "edges": edges,
        });
        if let Some(candidate) = candidate(RecipeCandidate {
            recipe: "relationship_graph",
            scene_id: scene.scene_id.as_str(),
            text,
            start,
            end,
            title: "Relationship graph".to_string(),
            suggested_visual: "investigation_board",
            priority: 0.82,
            entity_ids: entity_ids.into_iter().collect(),
            covered_fact_ids: covered,
            payload,
            dedup_key: format!("relationship_graph:{}:{start}:{end}", scene.scene_id),
            rationale: "asserted person relations with two grounded subjects",
        }) {
            candidates.push(candidate);
        }
    }
}

fn push_role_lower_thirds(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    listed_professions: &HashSet<(String, String, usize, usize)>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for role in &scene.person_roles {
            let Some(role_evidence) = grounded(text, role.start_byte, role.end_byte) else {
                continue;
            };
            if role.status != semantic_core::RoleStatus::Asserted {
                continue;
            }
            // The attribute card already names this role: one fact, one card.
            if listed_professions.contains(&(
                scene.scene_id.clone(),
                role.person_id.clone(),
                role.start_byte,
                role.end_byte,
            )) {
                continue;
            }
            let Some(person) = ties(identities, &role.person_id) else {
                continue;
            };
            let organization = role
                .organization_id
                .as_ref()
                .and_then(|id| ties(identities, id));
            let temporal = role
                .temporal
                .as_ref()
                .and_then(|temporal| temporal.normalized.clone());
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "role_lower_third",
                "person_id": role.person_id,
                "person_label": person,
                "role_code": role.role_code,
                "role_surface": role.role_surface,
                "gender": role.gender,
                "organization_id": role.organization_id,
                "organization_label": organization,
                "place_id": role.place_id,
                "date": temporal,
                "tense": role.tense,
                "status": role.status,
                "evidence": role_evidence,
            });
            let mut entity_ids = vec![role.person_id.clone()];
            if let Some(organization_id) = &role.organization_id {
                entity_ids.push(organization_id.clone());
            }
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "role_lower_third",
                scene_id: scene.scene_id.as_str(),
                text,
                start: role.start_byte,
                end: role.end_byte,
                title: format!("{person} — {}", role.role_surface),
                suggested_visual: "role_lower_third",
                priority: 0.8,
                entity_ids,
                covered_fact_ids: vec![role.id.clone()],
                payload,
                dedup_key: format!("role_lower_third:{}:{}", scene.scene_id, role.id),
                rationale: "asserted person role with a grounded person and role slot",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

fn push_route_maps(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let places: HashMap<&str, &semantic_core::Place> = scene
            .places
            .iter()
            .map(|place| (place.id.as_str(), place))
            .collect();
        for route in &scene.routes {
            if route.status != "asserted" || route.stops.len() < 2 {
                continue;
            }
            let mut stops = Vec::new();
            let mut entity_ids = HashSet::new();
            let mut complete = true;
            for stop in &route.stops {
                let Some(place) = places.get(stop.place_id.as_str()) else {
                    complete = false;
                    break;
                };
                let (Some(lat), Some(lon)) = (place.lat, place.lon) else {
                    complete = false;
                    break;
                };
                entity_ids.insert(place.entity_local_id.clone());
                stops.push(json!({
                    "place_id": place.id,
                    "entity_id": place.entity_local_id,
                    "name": place.name,
                    "kb_id": place.kb_id,
                    "lat": lat,
                    "lon": lon,
                    "role": stop.role,
                }));
            }
            if !complete || stops.len() < 2 {
                continue;
            }
            if let Some(subject_id) = &route.subject_id {
                if identities.contains_key(subject_id) {
                    entity_ids.insert(subject_id.clone());
                }
            }
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "route_map",
                "mode": route.mode,
                "subject_id": route.subject_id,
                "straight_line_km": route.straight_line_km,
                "stops": stops,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "route_map",
                scene_id: scene.scene_id.as_str(),
                text,
                start: route.start_byte,
                end: route.end_byte,
                title: "Route map".to_string(),
                suggested_visual: "route_map",
                priority: 0.86,
                entity_ids: entity_ids.into_iter().collect(),
                covered_fact_ids: vec![route.id.clone()],
                payload,
                dedup_key: format!("route_map:{}:{}", scene.scene_id, route.id),
                rationale: "route has at least two linked waypoints with coordinates",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

fn push_choropleths(
    scenes: &[SceneManifest],
    texts: &HashMap<&str, &str>,
    chart_spans: &HashMap<String, Vec<(usize, usize)>>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let empty = Vec::new();
        let covered_chart = chart_spans.get(&scene.scene_id).unwrap_or(&empty);
        let mut items = Vec::new();
        let mut entity_ids = HashSet::new();
        let mut covered = Vec::new();
        let mut start = usize::MAX;
        let mut end = 0;
        let mut places_seen = HashSet::new();
        for quantity in &scene.quantities {
            let Some(value) = quantity.value else {
                continue;
            };
            if quantity.status != "asserted"
                || grounded(text, quantity.start_byte, quantity.end_byte)
                    != Some(quantity.evidence.as_str())
                || overlaps(covered_chart, quantity.start_byte, quantity.end_byte)
                || quantity.dimension == "rank"
                || quantity.dimension == "growth_rate"
            {
                continue;
            }
            // The place must be grounded in the same sentence and carry
            // coordinates; a country name without a linked coordinate cannot
            // become a choropleth cell.
            let candidates: Vec<&semantic_core::Place> = scene
                .places
                .iter()
                .filter(|place| {
                    place.sentence_index == quantity.sentence_index
                        && place.lat.is_some()
                        && place.lon.is_some()
                })
                .collect();
            // A setting place precedes its value ("in France ... 12%"). Only
            // fall back to a following place when the sentence has no setting.
            let place = candidates
                .iter()
                .copied()
                .filter(|place| place.end_byte <= quantity.start_byte)
                .min_by_key(|place| quantity.start_byte - place.end_byte)
                .or_else(|| {
                    candidates
                        .into_iter()
                        .min_by_key(|place| place.start_byte.abs_diff(quantity.end_byte))
                });
            let Some(place) = place else {
                continue;
            };
            if !places_seen.insert(place.id.clone()) {
                continue;
            }
            entity_ids.insert(place.entity_local_id.clone());
            start = start.min(quantity.start_byte);
            end = end.max(quantity.end_byte);
            covered.push(quantity.id.clone());
            items.push(json!({
                "place_id": place.id,
                "entity_id": place.entity_local_id,
                "name": place.name,
                "kb_id": place.kb_id,
                "lat": place.lat,
                "lon": place.lon,
                "value": value,
                "unit": quantity.unit,
                "unit_surface": quantity.unit_surface,
                "dimension": quantity.dimension,
                "quantity_id": quantity.id,
                "start_byte": quantity.start_byte,
                "end_byte": quantity.end_byte,
                "evidence": quantity.evidence,
            }));
        }
        if items.len() < 2 {
            continue;
        }
        let payload = json!({
            "schema": RECIPE_SCHEMA,
            "recipe": "choropleth_value_map",
            "items": items,
        });
        if let Some(candidate) = candidate(RecipeCandidate {
            recipe: "choropleth_value_map",
            scene_id: scene.scene_id.as_str(),
            text,
            start,
            end,
            title: "Choropleth".to_string(),
            suggested_visual: "choropleth_value_map",
            priority: 0.84,
            entity_ids: entity_ids.into_iter().collect(),
            covered_fact_ids: covered,
            payload,
            dedup_key: format!("choropleth_value_map:{}:{start}:{end}", scene.scene_id),
            rationale: "at least two coordinate-bound quantities outside a chart",
        }) {
            candidates.push(candidate);
        }
    }
}

mod lifespans;
use lifespans::*;
fn push_counter_odometers(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    chart_spans: &HashMap<String, Vec<(usize, usize)>>,
    attribute_spans: &HashSet<(String, usize, usize)>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let empty = Vec::new();
        let covered_chart = chart_spans.get(&scene.scene_id).unwrap_or(&empty);
        for quantity in &scene.quantities {
            let Some(value) = quantity.value else {
                continue;
            };
            if quantity.status != "asserted"
                || !matches!(
                    quantity.dimension.as_str(),
                    "currency" | "percent" | "count" | "population"
                )
                || overlaps(covered_chart, quantity.start_byte, quantity.end_byte)
                || grounded(text, quantity.start_byte, quantity.end_byte)
                    != Some(quantity.evidence.as_str())
                || attribute_spans.contains(&(
                    scene.scene_id.clone(),
                    quantity.start_byte,
                    quantity.end_byte,
                ))
            {
                continue;
            }
            let fallback_subject = scene
                .entities
                .iter()
                .filter(|entity| {
                    entity.sentence_index == quantity.sentence_index
                        && entity.end_byte <= quantity.start_byte
                        && identities.contains_key(&entity.id)
                })
                .min_by_key(|entity| quantity.start_byte - entity.end_byte);
            let subject_id = quantity
                .subject_entity_id
                .as_ref()
                .or_else(|| fallback_subject.map(|entity| &entity.id));
            let Some(subject_id) = subject_id else {
                continue;
            };
            let Some(subject) = ties(identities, subject_id) else {
                continue;
            };
            // A counter is a single KPI, not one bar of an implicit series.
            let same_sentence = scene
                .quantities
                .iter()
                .filter(|other| {
                    other.sentence_index == quantity.sentence_index
                        && other.value.is_some()
                        && other.status == "asserted"
                })
                .count();
            if same_sentence != 1 {
                continue;
            }
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "counter_odometer",
                "subject_id": subject_id,
                "subject_label": subject,
                "value": value,
                "unit": quantity.unit,
                "unit_surface": quantity.unit_surface,
                "dimension": quantity.dimension,
                "qualifier": quantity.qualifier,
                "period": quantity.period,
                "quantity_id": quantity.id,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "counter_odometer",
                scene_id: scene.scene_id.as_str(),
                text,
                start: quantity.start_byte,
                end: quantity.end_byte,
                title: subject.clone(),
                suggested_visual: "counter_odometer",
                priority: 0.8,
                entity_ids: vec![subject_id.clone()],
                covered_fact_ids: vec![quantity.id.clone()],
                payload,
                dedup_key: format!("counter_odometer:{}:{}", scene.scene_id, quantity.id),
                rationale: "single asserted KPI with a grounded unit and subject",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

/// Exact value spans already represented by a grouped attribute card.
fn attribute_list_spans(candidates: &[ProductionCandidate]) -> HashSet<(String, usize, usize)> {
    let mut spans = HashSet::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.recipe == "entity_attribute_list")
    {
        let Some(attributes) = candidate
            .visual_payload
            .as_ref()
            .and_then(|payload| payload.get("attributes"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for attribute in attributes {
            let (Some(start), Some(end)) = (
                attribute
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64),
                attribute
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64),
            ) else {
                continue;
            };
            spans.insert((candidate.scene_id.clone(), start as usize, end as usize));
        }
    }
    spans
}

/// Roles already named by a grouped attribute card, keyed by the exact role
/// span so a second role of the same person keeps its own lower third.
fn profession_attribute_spans(
    candidates: &[ProductionCandidate],
) -> HashSet<(String, String, usize, usize)> {
    let mut spans = HashSet::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.recipe == "entity_attribute_list")
    {
        let Some(entity_id) = candidate.entity_ids.first() else {
            continue;
        };
        let Some(attributes) = candidate
            .visual_payload
            .as_ref()
            .and_then(|payload| payload.get("attributes"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for attribute in attributes {
            if attribute
                .get("predicate")
                .and_then(serde_json::Value::as_str)
                != Some("profession")
            {
                continue;
            }
            let (Some(start), Some(end)) = (
                attribute
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64),
                attribute
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64),
            ) else {
                continue;
            };
            spans.insert((
                candidate.scene_id.clone(),
                entity_id.clone(),
                start as usize,
                end as usize,
            ));
        }
    }
    spans
}

pub(super) fn push_visual_recipes(
    language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    events: &[EventFrame],
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let texts: HashMap<&str, &str> = sources
        .iter()
        .map(|scene| (scene.scene_id.as_str(), scene.text.as_str()))
        .collect();
    let covered_chart = chart_spans(candidates);
    push_entity_attribute_lists(language, scenes, identities, &texts, candidates);
    let attribute_spans = attribute_list_spans(candidates);
    let listed_professions = profession_attribute_spans(candidates);
    push_relationship_graphs(scenes, identities, &texts, candidates);
    push_role_lower_thirds(scenes, identities, &texts, &listed_professions, candidates);
    push_route_maps(scenes, identities, &texts, candidates);
    push_quote_attributions(scenes, identities, &texts, candidates);
    push_source_highlights(scenes, identities, &texts, candidates);
    push_choropleths(scenes, &texts, &covered_chart, candidates);
    push_lifespans(scenes, events, identities, sources, &texts, candidates);
    push_counter_odometers(
        scenes,
        identities,
        &texts,
        &covered_chart,
        &attribute_spans,
        candidates,
    );
    push_relative_time_jumps(language, scenes, &texts, candidates);
    push_cause_effect_cards(scenes, &texts, &covered_chart, candidates);
    push_process_steppers(scenes, &texts, &covered_chart, candidates);
}

/// `+3 ANNI` / `-48 ORE`: the display form of a resolved relative date. The
/// engine owns the arithmetic; this label only makes the payload renderable
/// without re-parsing the source surface downstream.
fn relative_delta_label(language: &str, direction: &str, amount: i64, unit: &str) -> String {
    let sign = match direction {
        "after" => "+",
        "before" => "-",
        _ => "",
    };
    let singular = amount == 1;
    let unit = match (language, unit, singular) {
        ("it", "day", true) => "GIORNO",
        ("it", "day", false) => "GIORNI",
        ("it", "week", true) => "SETTIMANA",
        ("it", "week", false) => "SETTIMANE",
        ("it", "month", true) => "MESE",
        ("it", "month", false) => "MESI",
        ("it", "year", true) => "ANNO",
        ("it", "year", false) => "ANNI",
        ("it", "decade", true) => "DECENNIO",
        ("it", "decade", false) => "DECENNI",
        ("it", "century", true) => "SECOLO",
        ("it", "century", false) => "SECOLI",
        ("en", "day", true) => "DAY",
        ("en", "day", false) => "DAYS",
        ("en", "week", true) => "WEEK",
        ("en", "week", false) => "WEEKS",
        ("en", "month", true) => "MONTH",
        ("en", "month", false) => "MONTHS",
        ("en", "year", true) => "YEAR",
        ("en", "year", false) => "YEARS",
        ("en", "decade", true) => "DECADE",
        ("en", "decade", false) => "DECADES",
        ("en", "century", true) => "CENTURY",
        ("en", "century", false) => "CENTURIES",
        (_, "day", true) => "DAY",
        (_, "day", false) => "DAYS",
        (_, "week", true) => "WEEK",
        (_, "week", false) => "WEEKS",
        (_, "month", true) => "MONTH",
        (_, "month", false) => "MONTHS",
        (_, "year", true) => "YEAR",
        (_, "year", false) => "YEARS",
        (_, "decade", true) => "DECADE",
        (_, "decade", false) => "DECADES",
        (_, "century", true) => "CENTURY",
        (_, "century", false) => "CENTURIES",
        _ => return format!("{sign}{} {}", amount.abs(), unit.to_uppercase()),
    };
    format!("{sign}{} {unit}", amount.abs())
}

/// Relative dates are already resolved by the semantic-core analyzer. This
/// recipe carries the resolved ISO value and the spoken delta, so a planner can
/// render a time jump without re-deriving either.
fn push_relative_time_jumps(
    language: &str,
    scenes: &[SceneManifest],
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for fact in &scene.temporal_facts {
            let Some(relative) = fact.relative.as_ref() else {
                continue;
            };
            let Some(resolved) = fact.normalized.as_deref() else {
                continue;
            };
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "relative_time_jump",
                "surface": fact.surface,
                "delta_label": relative_delta_label(
                    language,
                    relative.direction.as_str(),
                    relative.amount,
                    relative.unit.as_str(),
                ),
                "resolved_date": resolved,
                "direction": relative.direction,
                "amount": relative.amount,
                "unit": relative.unit,
                "start_byte": fact.start_byte,
                "end_byte": fact.end_byte,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "relative_time_jump",
                scene_id: scene.scene_id.as_str(),
                text,
                start: fact.start_byte,
                end: fact.end_byte,
                title: "Salto temporale".to_string(),
                suggested_visual: "relative_time_jump",
                priority: 0.78,
                entity_ids: Vec::new(),
                covered_fact_ids: Vec::new(),
                payload,
                dedup_key: format!(
                    "relative_time_jump:{}:{}:{}",
                    scene.scene_id, fact.start_byte, fact.end_byte
                ),
                rationale: "resolved relative date with a grounded source surface",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

/// A cause-effect group is exactly two grounded members; a single missing slot
/// is no card. The chart span guard keeps the sentence card from covering a
/// chart that already represents the same numbers.
fn push_cause_effect_cards(
    scenes: &[SceneManifest],
    texts: &HashMap<&str, &str>,
    chart_spans: &HashMap<String, Vec<(usize, usize)>>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let empty = Vec::new();
        let covered_chart = chart_spans.get(&scene.scene_id).unwrap_or(&empty);
        for group in &scene.semantic_groups {
            if group.group_type != "cause_effect" || group.members.len() != 2 {
                continue;
            }
            if group
                .members
                .iter()
                .any(|member| !member.scene_id.is_empty() && member.scene_id != scene.scene_id)
            {
                continue;
            }
            let start = group
                .members
                .iter()
                .map(|member| member.start_byte)
                .min()
                .unwrap_or(0);
            let end = group
                .members
                .iter()
                .map(|member| member.end_byte)
                .max()
                .unwrap_or(start);
            if overlaps(covered_chart, start, end) {
                continue;
            }
            let cause = &group.members[0];
            let effect = &group.members[1];
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "cause_effect_card",
                "cause_text": cause.text,
                "effect_text": effect.text,
                "cause_evidence": cause.evidence,
                "effect_evidence": effect.evidence,
                "cause_start_byte": cause.start_byte,
                "cause_end_byte": cause.end_byte,
                "effect_start_byte": effect.start_byte,
                "effect_end_byte": effect.end_byte,
                "start_byte": start,
                "end_byte": end,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "cause_effect_card",
                scene_id: scene.scene_id.as_str(),
                text,
                start,
                end,
                title: group.title.clone(),
                suggested_visual: "cause_effect_card",
                priority: 0.8,
                entity_ids: Vec::new(),
                covered_fact_ids: Vec::new(),
                payload,
                dedup_key: format!("cause_effect_card:{}:{}", scene.scene_id, group.id),
                rationale: "asserted causal relation with a grounded cause and effect",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

/// Process and step-sequence groups already carry ordered, grounded members.
/// The stepper recipe forwards that order verbatim; it never invents a step.
fn push_process_steppers(
    scenes: &[SceneManifest],
    texts: &HashMap<&str, &str>,
    chart_spans: &HashMap<String, Vec<(usize, usize)>>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        let empty = Vec::new();
        let covered_chart = chart_spans.get(&scene.scene_id).unwrap_or(&empty);
        for group in &scene.semantic_groups {
            if !matches!(group.group_type.as_str(), "process" | "step_sequence")
                || !(2..=6).contains(&group.members.len())
            {
                continue;
            }
            // A cross-scene process is anchored in its first scene; its later
            // members speak in other scenes and cannot be timed here.
            if group
                .members
                .iter()
                .any(|member| !member.scene_id.is_empty() && member.scene_id != scene.scene_id)
            {
                continue;
            }
            let start = group
                .members
                .iter()
                .map(|member| member.start_byte)
                .min()
                .unwrap_or(0);
            let end = group
                .members
                .iter()
                .map(|member| member.end_byte)
                .max()
                .unwrap_or(start);
            if overlaps(covered_chart, start, end) {
                continue;
            }
            let steps: Vec<serde_json::Value> = group
                .members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    json!({
                        "index": index + 1,
                        "text": member.text,
                        "evidence": member.evidence,
                        "start_byte": member.start_byte,
                        "end_byte": member.end_byte,
                    })
                })
                .collect();
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "process_stepper",
                "start_byte": start,
                "end_byte": end,
                "steps": steps,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "process_stepper",
                scene_id: scene.scene_id.as_str(),
                text,
                start,
                end,
                title: group.title.clone(),
                suggested_visual: "process_stepper",
                priority: 0.79,
                entity_ids: Vec::new(),
                covered_fact_ids: Vec::new(),
                payload,
                dedup_key: format!("process_stepper:{}:{}", scene.scene_id, group.id),
                rationale: "ordered grounded process with at least two members",
            }) {
                candidates.push(candidate);
            }
        }
    }
}
