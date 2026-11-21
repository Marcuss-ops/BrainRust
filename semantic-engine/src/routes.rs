//! Conservative within-sentence route extraction from explicit movement
//! language and grounded place mentions.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use semantic_core::{
    CrossSceneRoute, CrossSceneRouteStop, Place, Route, RouteEvidence, RouteStop, SceneManifest,
    SentenceSpan,
};
use serde::Deserialize;
use visualner::VisualEntity;

const CUES_JSON: &str = include_str!("../../lexicon/route_cues.json");

#[derive(Deserialize)]
struct CueFile {
    languages: BTreeMap<String, RouteCues>,
}

#[derive(Deserialize)]
struct RouteCues {
    origin: Vec<String>,
    destination: Vec<String>,
    motion: Vec<String>,
    air: Vec<String>,
    road: Vec<String>,
    rail: Vec<String>,
    sea: Vec<String>,
    #[serde(default)]
    waypoint: Vec<String>,
}

fn cues(language: &str) -> Option<&'static RouteCues> {
    static CUES: OnceLock<CueFile> = OnceLock::new();
    CUES.get_or_init(|| serde_json::from_str(CUES_JSON).expect("route cue data must be valid"))
        .languages
        .get(language.split('-').next().unwrap_or(language))
}

fn prefix_cue_start(prefix: &str, terms: &[String]) -> Option<usize> {
    let trimmed = prefix.trim_end();
    let lower = trimmed.to_lowercase();
    terms
        .iter()
        .filter_map(|term| {
            let term = term.to_lowercase();
            let start = lower.strip_suffix(&term)?.len();
            (start == 0
                || lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| ch.is_whitespace() || ch == '\''))
            .then_some(start)
        })
        .max_by_key(|start| *start)
}

fn contains_term(text: &str, term: &str) -> bool {
    let lower = text.to_lowercase();
    let term = term.to_lowercase();
    lower.match_indices(&term).any(|(start, matched)| {
        let end = start + matched.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn has_any(text: &str, terms: &[String]) -> bool {
    terms.iter().any(|term| contains_term(text, term))
}

fn first_term_offset(text: &str, terms: &[String]) -> Option<usize> {
    let lower = text.to_lowercase();
    terms
        .iter()
        .filter_map(|term| {
            let term = term.to_lowercase();
            lower.match_indices(&term).find_map(|(start, matched)| {
                let end = start + matched.len();
                let before = lower[..start].chars().next_back();
                let after = lower[end..].chars().next();
                (!before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric))
                .then_some(start)
            })
        })
        .min()
}

fn term_offsets(text: &str, terms: &[String]) -> Vec<usize> {
    let lower = text.to_lowercase();
    let mut offsets: Vec<usize> = terms
        .iter()
        .flat_map(|term| {
            let term = term.to_lowercase();
            let lower = &lower;
            lower
                .match_indices(&term)
                .filter_map(move |(start, matched)| {
                    let end = start + matched.len();
                    let before = lower[..start].chars().next_back();
                    let after = lower[end..].chars().next();
                    (!before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric))
                    .then_some(start)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

fn same_clause(text: &str, left: usize, right: usize) -> bool {
    text.get(left.min(right)..left.max(right))
        .is_some_and(|gap| {
            !gap.chars()
                .any(|ch| matches!(ch, ',' | ';' | ':' | '!' | '?'))
        })
}

fn status(evidence: &str) -> &'static str {
    let lower = evidence.to_lowercase();
    if [
        " not ", " never ", " non ", " mai ", " nunca ", " nie ", " не ",
    ]
    .iter()
    .any(|cue| format!(" {lower} ").contains(cue))
    {
        "negated"
    } else if [" would ", " might ", " hypothetically ", " forse ", " se "]
        .iter()
        .any(|cue| format!(" {lower} ").contains(cue))
    {
        "uncertain"
    } else {
        "asserted"
    }
}

fn mode(text: &str, cues: &RouteCues) -> &'static str {
    if has_any(text, &cues.air) {
        "air"
    } else if has_any(text, &cues.road) {
        "road"
    } else if has_any(text, &cues.rail) {
        "rail"
    } else if has_any(text, &cues.sea) {
        "sea"
    } else {
        "unknown"
    }
}

fn distance_km(origin: &Place, destination: &Place) -> Option<f64> {
    let (lat1, lon1, lat2, lon2) = (origin.lat?, origin.lon?, destination.lat?, destination.lon?);
    let (lat1, lat2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    Some(6371.0088 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt()))
}

#[cfg(test)]
fn extract(
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

/// Join asserted route hops across adjacent scenes only when the same
/// grounded person continues through the same linked endpoint.
pub fn merge_cross_scene_routes(scenes: &[SceneManifest]) -> Vec<CrossSceneRoute> {
    struct Pending {
        route: CrossSceneRoute,
        first_scene_index: usize,
        last_scene_index: usize,
        last_destination_kb_id: String,
        first_place: Place,
        last_place: Place,
        hop_count: usize,
    }

    let mut pending: Vec<Pending> = Vec::new();
    for (scene_index, scene) in scenes.iter().enumerate() {
        let mut routes = scene.routes.iter().collect::<Vec<_>>();
        routes.sort_by_key(|route| (route.start_byte, route.end_byte, route.id.as_str()));
        for route in routes {
            let Some(subject_id) = route.subject_id.as_ref() else {
                continue;
            };
            if route.status != "asserted" || route.stops.len() < 2 {
                continue;
            }
            let Some(origin) = scene.places.iter().find(|place| {
                place.id == route.stops[0].place_id
                    && place.link_status == "linked"
                    && place.kb_id.is_some()
            }) else {
                continue;
            };
            let Some(destination) = scene.places.iter().find(|place| {
                place.id == route.stops.last().expect("length checked").place_id
                    && place.link_status == "linked"
                    && place.kb_id.is_some()
            }) else {
                continue;
            };
            let Some(origin_kb_id) = origin.kb_id.as_ref() else {
                continue;
            };

            let matching: Vec<usize> = pending
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.route.subject_id == *subject_id
                        && item.last_scene_index + 1 == scene_index
                        && item.last_destination_kb_id == *origin_kb_id
                })
                .map(|(index, _)| index)
                .collect();

            let evidence = RouteEvidence {
                scene_id: scene.scene_id.clone(),
                route_id: route.id.clone(),
                start_byte: route.start_byte,
                end_byte: route.end_byte,
                evidence: route.evidence.clone(),
            };
            if matching.len() == 1 {
                let item = &mut pending[matching[0]];
                if let Some(join) = item.route.stops.last_mut() {
                    join.role = "waypoint".to_string();
                }
                item.route
                    .stops
                    .extend(route.stops.iter().skip(1).map(|stop| CrossSceneRouteStop {
                        scene_id: scene.scene_id.clone(),
                        place_id: stop.place_id.clone(),
                        role: stop.role.clone(),
                    }));
                item.route.evidence.push(evidence);
                item.route.mode = if item.route.mode == route.mode {
                    route.mode.clone()
                } else {
                    "unknown".to_string()
                };
                item.last_scene_index = scene_index;
                item.last_destination_kb_id = destination.kb_id.clone().unwrap_or_default();
                item.last_place = destination.clone();
                item.hop_count += 1;
                item.route.straight_line_km = distance_km(&item.first_place, &item.last_place);
            } else {
                pending.push(Pending {
                    route: CrossSceneRoute {
                        id: String::new(),
                        subject_id: subject_id.clone(),
                        mode: route.mode.clone(),
                        stops: route
                            .stops
                            .iter()
                            .map(|stop| CrossSceneRouteStop {
                                scene_id: scene.scene_id.clone(),
                                place_id: stop.place_id.clone(),
                                role: stop.role.clone(),
                            })
                            .collect(),
                        straight_line_km: None,
                        status: "asserted".to_string(),
                        evidence: vec![evidence],
                    },
                    first_scene_index: scene_index,
                    last_scene_index: scene_index,
                    last_destination_kb_id: destination.kb_id.clone().unwrap_or_default(),
                    first_place: origin.clone(),
                    last_place: destination.clone(),
                    hop_count: 1,
                });
            }
        }
    }

    let mut routes: Vec<_> = pending
        .into_iter()
        .filter(|item| item.hop_count > 1)
        .collect();
    routes.sort_by_key(|item| {
        (
            item.first_scene_index,
            item.route
                .evidence
                .first()
                .map(|e| e.start_byte)
                .unwrap_or(0),
            item.route.subject_id.clone(),
        )
    });
    routes
        .into_iter()
        .enumerate()
        .map(|(index, mut item)| {
            item.route.id = format!("cr_{:04}", index + 1);
            item.route
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, start: usize) -> VisualEntity {
        VisualEntity {
            text: name.to_string(),
            r#type: "PERSON".to_string(),
            score: 1.0,
            start,
            end: start + name.len(),
            evidence: name.to_string(),
        }
    }

    fn place(id: &str, name: &str, start: usize, lat: f64, lon: f64) -> Place {
        Place {
            id: format!("pl_{id}"),
            entity_local_id: format!("loc_{id}"),
            kb_id: Some(format!("wd:{id}")),
            link_status: "linked".to_string(),
            kind: "city".to_string(),
            name: name.to_string(),
            lat: Some(lat),
            lon: Some(lon),
            parent_ids: Vec::new(),
            role: "mention".to_string(),
            start_byte: start,
            end_byte: start + name.len(),
            sentence_index: 0,
            evidence: name.to_string(),
        }
    }

    #[test]
    fn explicit_english_motion_emits_ordered_route_and_distance() {
        let source = "They drove from Paris to Rome.";
        let places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        let routes = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        );
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].mode, "road");
        assert_eq!(routes[0].stops[0].place_id, "pl_90");
        assert_eq!(routes[0].stops[1].place_id, "pl_220");
        assert!(routes[0].straight_line_km.unwrap() > 100.0);
        assert!(routes[0].evidence.starts_with("drove from"));
        assert_eq!(
            &source[routes[0].start_byte..routes[0].end_byte],
            routes[0].evidence
        );
    }

    #[test]
    fn explicit_italian_motion_uses_localized_route_cues() {
        let source = "Viaggiò da Roma verso Parigi.";
        let places = vec![
            place(
                "roma",
                "Roma",
                source.find("Roma").unwrap(),
                41.9028,
                12.4964,
            ),
            place(
                "parigi",
                "Parigi",
                source.find("Parigi").unwrap(),
                48.8566,
                2.3522,
            ),
        ];
        let routes = extract(
            source,
            "it",
            &semantic_core::split_sentences(source, "it", None),
            &places,
        );
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].stops[0].role, "origin");
        assert_eq!(routes[0].stops[1].role, "destination");
        assert!(routes[0].straight_line_km.is_some());
    }

    #[test]
    fn explicit_waypoints_are_kept_in_source_order() {
        let source = "They drove from Paris via Lyon and Dijon to Rome.";
        let places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place("493", "Lyon", source.find("Lyon").unwrap(), 45.7640, 4.8357),
            place(
                "181",
                "Dijon",
                source.find("Dijon").unwrap(),
                47.3220,
                5.0415,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        let routes = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        );
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0]
                .stops
                .iter()
                .map(|stop| stop.place_id.as_str())
                .collect::<Vec<_>>(),
            ["pl_90", "pl_493", "pl_181", "pl_220"]
        );
        assert_eq!(routes[0].stops[1].role, "waypoint");
        assert_eq!(routes[0].stops[2].role, "waypoint");
        assert!(routes[0].straight_line_km.is_some());
    }

    #[test]
    fn an_ambiguous_waypoint_makes_the_whole_route_uncertain() {
        let source = "They drove from Paris via Lyon to Rome.";
        let mut waypoint = place("493", "Lyon", source.find("Lyon").unwrap(), 45.7640, 4.8357);
        waypoint.link_status = "ambiguous".to_string();
        waypoint.kb_id = None;
        waypoint.lat = None;
        waypoint.lon = None;
        let places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            waypoint,
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        let routes = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        );
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].stops[1].role, "waypoint");
        assert_eq!(routes[0].status, "uncertain");
        assert!(routes[0].straight_line_km.is_none());
    }

    #[test]
    fn consecutive_hops_merge_only_for_the_same_grounded_subject() {
        let source = "Marco Rossi drove from Paris to Lyon. Marco Rossi drove from Lyon to Rome.";
        let sentences = semantic_core::split_sentences(source, "en", None);
        let mut places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place("493", "Lyon", source.find("Lyon").unwrap(), 45.7640, 4.8357),
            place(
                "493",
                "Lyon",
                source.rfind("Lyon").unwrap(),
                45.7640,
                4.8357,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        for place in &mut places {
            place.sentence_index = usize::from(place.start_byte >= sentences[1].start_byte);
        }
        let entities = vec![person("Marco Rossi", 0), person("Marco Rossi", 38)];
        let routes = extract_with_people(
            source,
            "en",
            &sentences,
            &places,
            &entities,
            &["person:marco".to_string(), "person:marco".to_string()],
        );
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].subject_id.as_deref(), Some("person:marco"));
        assert_eq!(
            routes[0]
                .stops
                .iter()
                .map(|stop| stop.place_id.as_str())
                .collect::<Vec<_>>(),
            ["pl_90", "pl_493", "pl_220"]
        );
        assert_eq!(routes[0].stops[1].role, "waypoint");
        assert_eq!(
            &source[routes[0].start_byte..routes[0].end_byte],
            routes[0].evidence
        );
        assert!(routes[0].straight_line_km.unwrap() > 100.0);

        let distinct = extract_with_people(
            source,
            "en",
            &sentences,
            &places,
            &entities,
            &["person:marco".to_string(), "person:other".to_string()],
        );
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn route_pattern_without_motion_is_not_emitted() {
        let source = "From Paris to Rome.";
        let places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        assert!(extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        )
        .is_empty());
    }

    #[test]
    fn movement_verb_in_a_separate_clause_does_not_create_a_route() {
        let source = "From Paris to Rome; they drove home.";
        let places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        assert!(extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        )
        .is_empty());
    }

    #[test]
    fn negated_and_ambiguous_routes_have_no_distance() {
        let source = "They never drove from Paris to Rome.";
        let mut places = vec![
            place(
                "90",
                "Paris",
                source.find("Paris").unwrap(),
                48.8566,
                2.3522,
            ),
            place(
                "220",
                "Rome",
                source.find("Rome").unwrap(),
                41.9028,
                12.4964,
            ),
        ];
        let routes = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &places,
        );
        assert_eq!(routes[0].status, "negated");
        assert!(routes[0].evidence.starts_with("never drove"));
        assert!(routes[0].straight_line_km.is_none());

        let clear_source = "They drove from Paris to Rome.";
        places[0].start_byte = clear_source.find("Paris").unwrap();
        places[0].end_byte = places[0].start_byte + places[0].name.len();
        places[1].start_byte = clear_source.find("Rome").unwrap();
        places[1].end_byte = places[1].start_byte + places[1].name.len();
        places[1].link_status = "ambiguous".to_string();
        let routes = extract(
            clear_source,
            "en",
            &semantic_core::split_sentences(clear_source, "en", None),
            &places,
        );
        assert_eq!(routes[0].status, "uncertain");
        assert!(routes[0].straight_line_km.is_none());
    }

    #[test]
    fn route_cue_packs_cover_all_supported_languages() {
        for language in [
            "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
        ] {
            let language_cues = cues(language).expect("route cue pack required");
            assert!(
                !language_cues.origin.is_empty(),
                "origin cues for {language}"
            );
            assert!(
                !language_cues.destination.is_empty(),
                "destination cues for {language}"
            );
            assert!(
                !language_cues.motion.is_empty(),
                "motion cues for {language}"
            );
            assert!(
                !language_cues.waypoint.is_empty(),
                "waypoint cues for {language}"
            );
        }
    }
}
