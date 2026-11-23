use super::*;

pub(super) struct LifespanPoint {
    scene_id: String,
    start: usize,
    end: usize,
    year: i32,
    date: String,
    kind: String,
    event_id: Option<String>,
}

pub(super) fn is_life_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "birth" | "death" | "appointment_resignation" | "election" | "founding" | "trial_verdict"
    )
}

pub(super) fn life_event_year(event: &EventFrame) -> Option<i32> {
    event
        .time
        .as_ref()?
        .normalized_date
        .as_ref()?
        .get(..4)?
        .parse::<i32>()
        .ok()
}

pub(super) fn collect_event_lifespan_points(
    events: &[EventFrame],
    identities: &BTreeMap<String, IdentityState>,
    points: &mut BTreeMap<String, Vec<LifespanPoint>>,
    labels: &mut BTreeMap<String, String>,
) {
    for event in events {
        if !is_life_event(&event.event_type) {
            continue;
        }
        let Some(year) = life_event_year(event) else {
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
                .push(LifespanPoint {
                    scene_id: evidence.scene_id.clone(),
                    start: evidence.start_byte,
                    end: evidence.end_byte,
                    year,
                    date: event
                        .time
                        .as_ref()
                        .and_then(|time| time.normalized_date.clone())
                        .unwrap_or_default(),
                    kind: event.event_type.clone(),
                    event_id: Some(event.id.clone()),
                });
        }
    }
}

/// A second mention of the same sentence can lose its participants
/// ("... was born in 1980 and died in 2020": the death trigger has only a
/// date). Attach it to the unique person already grounded on the same
/// verbatim evidence span.
pub(super) fn attach_unowned_lifespan_points(
    events: &[EventFrame],
    identities: &BTreeMap<String, IdentityState>,
    points: &mut BTreeMap<String, Vec<LifespanPoint>>,
    labels: &mut BTreeMap<String, String>,
) {
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
        if !is_life_event(&event.event_type) || !event.participants.is_empty() {
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
        points.entry(person_id).or_default().push(LifespanPoint {
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

pub(super) fn collect_scene_lifespan_points(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    points: &mut BTreeMap<String, Vec<LifespanPoint>>,
    labels: &mut BTreeMap<String, String>,
) {
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
                .push(LifespanPoint {
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
                .push(LifespanPoint {
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
}

pub(super) fn emit_lifespan_candidates(
    points: BTreeMap<String, Vec<LifespanPoint>>,
    labels: &BTreeMap<String, String>,
    sources: &[SceneInput],
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
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

pub(super) fn push_lifespans(
    scenes: &[SceneManifest],
    events: &[EventFrame],
    identities: &BTreeMap<String, IdentityState>,
    sources: &[SceneInput],
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let mut points: BTreeMap<String, Vec<LifespanPoint>> = BTreeMap::new();
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    collect_event_lifespan_points(events, identities, &mut points, &mut labels);
    attach_unowned_lifespan_points(events, identities, &mut points, &mut labels);
    collect_scene_lifespan_points(scenes, identities, &mut points, &mut labels);
    emit_lifespan_candidates(points, &labels, sources, texts, candidates);
}
