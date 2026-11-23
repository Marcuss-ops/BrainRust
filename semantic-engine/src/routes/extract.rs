//! Within-sentence route extraction and hop merging.

use semantic_core::{Place, Route, RouteStop, SentenceSpan};
use visualner::VisualEntity;

use super::cues::cues;
use super::cues::{
    distance_km, first_term_offset, has_any, mode, prefix_cue_start, same_clause, status,
    term_offsets,
};

#[cfg(test)]
pub(super) fn extract(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    places: &[Place],
) -> Vec<Route> {
    extract_inner(source, language, sentences, places, None)
}

pub fn extract_with_people(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    places: &[Place],
    people: &[VisualEntity],
    person_ids: &[String],
) -> Vec<Route> {
    extract_inner(
        source,
        language,
        sentences,
        places,
        Some((people, person_ids)),
    )
}

fn extract_inner(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    places: &[Place],
    people: Option<(&[VisualEntity], &[String])>,
) -> Vec<Route> {
    let Some(cues) = cues(language) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for sentence in sentences {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if !has_any(sentence_text, &cues.motion) {
            continue;
        }
        let mut sentence_places: Vec<&Place> = places
            .iter()
            .filter(|place| {
                place.sentence_index < sentences.len()
                    && place.start_byte >= sentence.start_byte
                    && place.end_byte <= sentence.end_byte
            })
            .collect();
        sentence_places.sort_by_key(|place| (place.start_byte, place.end_byte));
        for (origin_index, origin) in sentence_places.iter().enumerate() {
            let Some(origin_cue_start) = source
                .get(sentence.start_byte..origin.start_byte)
                .and_then(|prefix| prefix_cue_start(prefix, &cues.origin))
            else {
                continue;
            };
            for destination in sentence_places.iter().skip(origin_index + 1) {
                let Some(destination_cue_start) = source
                    .get(sentence.start_byte..destination.start_byte)
                    .and_then(|prefix| prefix_cue_start(prefix, &cues.destination))
                else {
                    continue;
                };
                if destination_cue_start < origin.end_byte.saturating_sub(sentence.start_byte) {
                    continue;
                }
                let origin_local = origin.start_byte - sentence.start_byte;
                let destination_local = destination.start_byte - sentence.start_byte;
                let Some(movement_offset) = term_offsets(sentence_text, &cues.motion)
                    .into_iter()
                    .find(|offset| {
                        same_clause(sentence_text, *offset, origin_local)
                            && same_clause(sentence_text, *offset, destination_local)
                    })
                else {
                    continue;
                };
                let assertion_offset = first_term_offset(
                    sentence_text,
                    &[
                        "not".to_string(),
                        "never".to_string(),
                        "non".to_string(),
                        "mai".to_string(),
                        "nunca".to_string(),
                        "nie".to_string(),
                        "не".to_string(),
                        "would".to_string(),
                        "might".to_string(),
                        "forse".to_string(),
                        "se".to_string(),
                    ],
                )
                .filter(|offset| {
                    same_clause(sentence_text, *offset, origin_local)
                        && same_clause(sentence_text, *offset, destination_local)
                })
                .unwrap_or(movement_offset);
                let start_byte = sentence.start_byte
                    + origin_cue_start.min(movement_offset).min(assertion_offset);
                let end_byte = destination.end_byte;
                let Some(evidence) = source.get(start_byte..end_byte) else {
                    continue;
                };
                let local_route_start = origin_cue_start.min(movement_offset).min(assertion_offset);
                let route_context = sentence_text
                    .get(local_route_start..destination_local)
                    .unwrap_or(sentence_text);
                let mut route_status = status(route_context).to_string();
                let same_place = origin.kb_id.is_some() && origin.kb_id == destination.kb_id;
                let mut stops = vec![RouteStop {
                    place_id: origin.id.clone(),
                    role: "origin".to_string(),
                }];
                let mut ambiguous =
                    origin.link_status == "ambiguous" || destination.link_status == "ambiguous";
                for waypoint in sentence_places
                    .iter()
                    .skip(origin_index + 1)
                    .take_while(|place| place.start_byte < destination.start_byte)
                {
                    let waypoint_local = waypoint.start_byte.saturating_sub(sentence.start_byte);
                    if !same_clause(sentence_text, movement_offset, waypoint_local) {
                        continue;
                    }
                    let between_origin_and_waypoint = source
                        .get(origin.end_byte..waypoint.start_byte)
                        .unwrap_or("");
                    if !has_any(between_origin_and_waypoint, &cues.waypoint) {
                        continue;
                    }
                    ambiguous |= waypoint.link_status == "ambiguous";
                    stops.push(RouteStop {
                        place_id: waypoint.id.clone(),
                        role: "waypoint".to_string(),
                    });
                }
                stops.push(RouteStop {
                    place_id: destination.id.clone(),
                    role: "destination".to_string(),
                });
                if same_place || ambiguous {
                    route_status = "uncertain".to_string();
                }
                let straight_line_km = if route_status == "asserted"
                    && origin.link_status == "linked"
                    && destination.link_status == "linked"
                    && !same_place
                {
                    distance_km(origin, destination)
                } else {
                    None
                };
                output.push(Route {
                    id: format!("rt_{:04}", output.len() + 1),
                    mode: mode(sentence_text, cues).to_string(),
                    subject_id: people.and_then(|(people, ids)| {
                        nearest_subject_id(
                            people,
                            ids,
                            sentence,
                            sentence.start_byte + movement_offset,
                        )
                    }),
                    stops,
                    straight_line_km,
                    status: route_status,
                    start_byte,
                    end_byte,
                    evidence: evidence.to_string(),
                });
            }
        }
    }
    merge_consecutive_hops(source, sentences, places, output)
}

fn nearest_subject_id(
    entities: &[VisualEntity],
    ids: &[String],
    sentence: &SentenceSpan,
    cue_start: usize,
) -> Option<String> {
    let mut people: Vec<(usize, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| {
            entity.r#type == "PERSON"
                && entity.start >= sentence.start_byte
                && entity.end <= sentence.end_byte
        })
        .filter_map(|(index, entity)| {
            ids.get(index)?;
            let distance = if entity.end <= cue_start {
                cue_start - entity.end
            } else if entity.start >= cue_start {
                entity.start.saturating_sub(cue_start)
            } else {
                0
            };
            (distance <= 100).then_some((index, distance))
        })
        .collect();
    people.sort_by_key(|(index, distance)| (*distance, entities[*index].start));
    if people.get(1).is_some_and(|(_, next)| *next == people[0].1) {
        return None;
    }
    people
        .first()
        .and_then(|(index, _)| ids.get(*index))
        .cloned()
}

struct PendingRoute {
    route: Route,
    last_hop_sentence: usize,
}

fn merge_consecutive_hops(
    source: &str,
    sentences: &[SentenceSpan],
    places: &[Place],
    routes: Vec<Route>,
) -> Vec<Route> {
    let mut merged: Vec<PendingRoute> = Vec::new();
    for route in routes {
        let sentence_index = sentences
            .iter()
            .position(|sentence| {
                route.start_byte >= sentence.start_byte && route.start_byte < sentence.end_byte
            })
            .unwrap_or(usize::MAX);
        let can_merge = sentence_index != usize::MAX
            && merged.last().is_some_and(|previous| {
                previous.last_hop_sentence.saturating_add(1) == sentence_index
                    && route.subject_id.is_some()
                    && route.subject_id == previous.route.subject_id
                    && route.status == "asserted"
                    && previous.route.status == "asserted"
                    && previous
                        .route
                        .stops
                        .last()
                        .map(|stop| stop.place_id.as_str())
                        == route.stops.first().map(|stop| stop.place_id.as_str())
            });
        if can_merge {
            let previous = merged.last_mut().expect("checked above");
            if let Some(evidence) = source.get(previous.route.start_byte..route.end_byte) {
                if let Some(join_stop) = previous.route.stops.last_mut() {
                    join_stop.role = "waypoint".to_string();
                }
                previous.route.end_byte = route.end_byte;
                previous.route.evidence = evidence.to_string();
                previous.route.stops.extend(route.stops.into_iter().skip(1));
                previous.route.mode = if previous.route.mode == route.mode {
                    route.mode
                } else {
                    "unknown".to_string()
                };
                previous.route.straight_line_km = route_distance(&previous.route, places);
                previous.last_hop_sentence = sentence_index;
                continue;
            }
        }
        merged.push(PendingRoute {
            route,
            last_hop_sentence: sentence_index,
        });
    }
    merged
        .into_iter()
        .enumerate()
        .map(|(index, mut pending)| {
            pending.route.id = format!("rt_{:04}", index + 1);
            pending.route
        })
        .collect()
}

fn route_distance(route: &Route, places: &[Place]) -> Option<f64> {
    if route.status != "asserted" {
        return None;
    }
    let origin_id = &route.stops.first()?.place_id;
    let destination_id = &route.stops.last()?.place_id;
    let origin = places.iter().find(|place| &place.id == origin_id)?;
    let destination = places.iter().find(|place| &place.id == destination_id)?;
    if origin.link_status != "linked" || destination.link_status != "linked" {
        return None;
    }
    distance_km(origin, destination)
}
