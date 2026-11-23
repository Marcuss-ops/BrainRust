//! Cross-scene route joining.

use semantic_core::{CrossSceneRoute, CrossSceneRouteStop, Place, RouteEvidence, SceneManifest};

use super::cues::distance_km;

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
