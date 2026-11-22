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

#[allow(clippy::too_many_arguments)]
fn candidate(
    recipe: &str,
    scene_id: &str,
    text: &str,
    start: usize,
    end: usize,
    title: String,
    suggested_visual: &str,
    priority: f32,
    entity_ids: Vec<String>,
    covered_fact_ids: Vec<String>,
    payload: serde_json::Value,
    dedup_key: String,
    rationale: &str,
) -> Option<ProductionCandidate> {
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
        if let Some(candidate) = candidate(
            "relationship_graph",
            &scene.scene_id,
            text,
            start,
            end,
            "Relationship graph".to_string(),
            "investigation_board",
            0.82,
            entity_ids.into_iter().collect(),
            covered,
            payload,
            format!("relationship_graph:{}:{start}:{end}", scene.scene_id),
            "asserted person relations with two grounded subjects",
        ) {
            candidates.push(candidate);
        }
    }
}

fn push_role_lower_thirds(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
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
            if let Some(candidate) = candidate(
                "role_lower_third",
                &scene.scene_id,
                text,
                role.start_byte,
                role.end_byte,
                format!("{person} — {}", role.role_surface),
                "role_lower_third",
                0.8,
                entity_ids,
                vec![role.id.clone()],
                payload,
                format!("role_lower_third:{}:{}", scene.scene_id, role.id),
                "asserted person role with a grounded person and role slot",
            ) {
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
            if let Some(candidate) = candidate(
                "route_map",
                &scene.scene_id,
                text,
                route.start_byte,
                route.end_byte,
                "Route map".to_string(),
                "route_map",
                0.86,
                entity_ids.into_iter().collect(),
                vec![route.id.clone()],
                payload,
                format!("route_map:{}:{}", scene.scene_id, route.id),
                "route has at least two linked waypoints with coordinates",
            ) {
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
        if let Some(candidate) = candidate(
            "choropleth_value_map",
            &scene.scene_id,
            text,
            start,
            end,
            "Choropleth".to_string(),
            "choropleth_value_map",
            0.84,
            entity_ids.into_iter().collect(),
            covered,
            payload,
            format!("choropleth_value_map:{}:{start}:{end}", scene.scene_id),
            "at least two coordinate-bound quantities outside a chart",
        ) {
            candidates.push(candidate);
        }
    }
}

fn push_lifespans(
    scenes: &[SceneManifest],
    events: &[EventFrame],
    identities: &BTreeMap<String, IdentityState>,
    sources: &[SceneInput],
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    #[derive(Clone)]
    struct Point {
        scene_id: String,
        start: usize,
        end: usize,
        year: i32,
        date: String,
        kind: String,
        event_id: Option<String>,
    }
    let life_events = [
        "birth",
        "death",
        "appointment_resignation",
        "election",
        "founding",
        "trial_verdict",
    ];
    let mut points: BTreeMap<String, Vec<Point>> = BTreeMap::new();
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for event in events {
        if !life_events.contains(&event.event_type.as_str()) {
            continue;
        }
        let Some(time) = event.time.as_ref() else {
            continue;
        };
        let Some(year) = time
            .normalized_date
            .as_ref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse::<i32>().ok())
        else {
            continue;
        };
        for participant in &event.participants {
            if !identities.contains_key(&participant.entity_id) {
                continue;
            }
            let Some(evidence) = event.evidence.first() else {
                continue;
            };
            labels.insert(
                participant.entity_id.clone(),
                ties(identities, &participant.entity_id).unwrap_or_default(),
            );
            points
                .entry(participant.entity_id.clone())
                .or_default()
                .push(Point {
                    scene_id: evidence.scene_id.clone(),
                    start: evidence.start_byte,
                    end: evidence.end_byte,
                    year,
                    date: time.normalized_date.clone().unwrap_or_default(),
                    kind: event.event_type.clone(),
                    event_id: Some(event.id.clone()),
                });
        }
    }
    // A second mention of the same sentence can lose its participants
    // ("... was born in 1980 and died in 2020": the death trigger has only a
    // date). Attach it to the unique person already grounded on the same
    // verbatim evidence span.
    let mut by_evidence: BTreeMap<(String, usize, usize), HashSet<String>> = BTreeMap::new();
    for event in events {
        let Some(evidence) = event.evidence.first() else {
            continue;
        };
        for participant in &event.participants {
            if identities.contains_key(&participant.entity_id) {
                by_evidence
                    .entry((
                        evidence.scene_id.clone(),
                        evidence.start_byte,
                        evidence.end_byte,
                    ))
                    .or_default()
                    .insert(participant.entity_id.clone());
            }
        }
    }
    for event in events {
        if !life_events.contains(&event.event_type.as_str()) || !event.participants.is_empty() {
            continue;
        }
        let (Some(time), Some(evidence)) = (event.time.as_ref(), event.evidence.first()) else {
            continue;
        };
        let Some(year) = time
            .normalized_date
            .as_ref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse::<i32>().ok())
        else {
            continue;
        };
        let Some(people) = by_evidence.get(&(
            evidence.scene_id.clone(),
            evidence.start_byte,
            evidence.end_byte,
        )) else {
            continue;
        };
        if people.len() != 1 {
            continue;
        }
        let person_id = people.iter().next().expect("one person").clone();
        labels.insert(
            person_id.clone(),
            ties(identities, &person_id).unwrap_or_default(),
        );
        points.entry(person_id).or_default().push(Point {
            scene_id: evidence.scene_id.clone(),
            start: evidence.start_byte,
            end: evidence.end_byte,
            year,
            date: time.normalized_date.clone().unwrap_or_default(),
            kind: event.event_type.clone(),
            event_id: Some(event.id.clone()),
        });
    }
    for scene in scenes {
        for role in &scene.person_roles {
            let Some(temporal) = role.temporal.as_ref() else {
                continue;
            };
            let Some(normalized) = temporal.normalized.as_ref() else {
                continue;
            };
            let Some(year) = normalized
                .get(..4)
                .and_then(|year| year.parse::<i32>().ok())
            else {
                continue;
            };
            if !identities.contains_key(&role.person_id) {
                continue;
            }
            labels.insert(
                role.person_id.clone(),
                ties(identities, &role.person_id).unwrap_or_default(),
            );
            points
                .entry(role.person_id.clone())
                .or_default()
                .push(Point {
                    scene_id: scene.scene_id.clone(),
                    start: role.start_byte,
                    end: role.end_byte,
                    year,
                    date: normalized.clone(),
                    kind: format!("role:{}", role.role_code),
                    event_id: None,
                });
        }
        for age in &scene.age_facts {
            let Some(year) = age.derived_birth_year else {
                continue;
            };
            if !identities.contains_key(&age.person_id) {
                continue;
            }
            labels.insert(
                age.person_id.clone(),
                ties(identities, &age.person_id).unwrap_or_default(),
            );
            points
                .entry(age.person_id.clone())
                .or_default()
                .push(Point {
                    scene_id: scene.scene_id.clone(),
                    start: age.start_byte,
                    end: age.end_byte,
                    year,
                    date: year.to_string(),
                    kind: "birth_year".to_string(),
                    event_id: None,
                });
        }
    }
    let scene_order: HashMap<&str, usize> = sources
        .iter()
        .enumerate()
        .map(|(index, scene)| (scene.scene_id.as_str(), index))
        .collect();
    for (person_id, mut person_points) in points {
        let distinct_years: HashSet<i32> = person_points.iter().map(|point| point.year).collect();
        if person_points.len() < 2 || distinct_years.len() < 2 {
            continue;
        }
        person_points.sort_by(|left, right| {
            left.year
                .cmp(&right.year)
                .then(left.scene_id.cmp(&right.scene_id))
                .then(left.start.cmp(&right.start))
        });
        let anchor = person_points
            .iter()
            .min_by_key(|point| {
                (
                    scene_order
                        .get(point.scene_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX),
                    point.start,
                )
            })
            .expect("non-empty points");
        let Some(text) = texts.get(anchor.scene_id.as_str()) else {
            continue;
        };
        if grounded(text, anchor.start, anchor.end).is_none() {
            continue;
        }
        let label = labels.get(&person_id).cloned().unwrap_or_default();
        let payload = json!({
            "schema": RECIPE_SCHEMA,
            "recipe": "lifespan_timeline",
            "person_id": person_id,
            "person_label": label,
            "items": person_points.iter().map(|point| json!({
                "scene_id": point.scene_id,
                "date": point.date,
                "year": point.year,
                "kind": point.kind,
                "event_id": point.event_id,
                "start_byte": point.start,
                "end_byte": point.end,
            })).collect::<Vec<_>>(),
        });
        if let Some(candidate) = candidate(
            "lifespan_timeline",
            &anchor.scene_id,
            text,
            anchor.start,
            anchor.end,
            format!("{label} — timeline"),
            "timeline",
            0.78,
            vec![person_id.clone()],
            person_points
                .iter()
                .filter_map(|point| point.event_id.clone())
                .collect(),
            payload,
            format!("lifespan_timeline:{person_id}"),
            "one grounded person with at least two distinct dated life events",
        ) {
            candidates.push(candidate);
        }
    }
}

fn push_counter_odometers(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
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
            if let Some(candidate) = candidate(
                "counter_odometer",
                &scene.scene_id,
                text,
                quantity.start_byte,
                quantity.end_byte,
                subject.clone(),
                "counter_odometer",
                0.8,
                vec![subject_id.clone()],
                vec![quantity.id.clone()],
                payload,
                format!("counter_odometer:{}:{}", scene.scene_id, quantity.id),
                "single asserted KPI with a grounded unit and subject",
            ) {
                candidates.push(candidate);
            }
        }
    }
}

pub(super) fn push_visual_recipes(
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
    push_relationship_graphs(scenes, identities, &texts, candidates);
    push_role_lower_thirds(scenes, identities, &texts, candidates);
    push_route_maps(scenes, identities, &texts, candidates);
    push_quote_attributions(scenes, identities, &texts, candidates);
    push_source_highlights(scenes, identities, &texts, candidates);
    push_choropleths(scenes, &texts, &covered_chart, candidates);
    push_lifespans(scenes, events, identities, sources, &texts, candidates);
    push_counter_odometers(scenes, identities, &texts, &covered_chart, candidates);
}
