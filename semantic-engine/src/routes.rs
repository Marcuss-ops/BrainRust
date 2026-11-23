//! Conservative within-sentence route extraction from explicit movement
//! language and grounded place mentions.

mod cues;

mod extract;

mod merge;

pub use extract::extract_with_people;
pub use merge::merge_cross_scene_routes;

#[cfg(test)]
use cues::cues;
#[cfg(test)]
use extract::extract;

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_core::Place;
    use visualner::VisualEntity;

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
