use super::*;
use crate::identity::Mention;

fn identity(id: &str, surface: &str, kind: &str, scene: &str) -> CanonicalIdentity {
    CanonicalIdentity {
        id: id.to_string(),
        canonical_id: format!("org:{}", surface.to_lowercase()),
        canonical_surface: surface.to_string(),
        kind: kind.to_string(),
        mentions: vec![Mention {
            scene_id: scene.to_string(),
            surface: surface.to_string(),
            start_byte: 0,
            end_byte: surface.len(),
        }],
    }
}

fn link_one(text: &str, surface: &str, kind: &str) -> LinkedEntity {
    let identities = vec![identity("org_01", surface, kind, "s1")];
    link_all("it", &[("s1", text)], &identities)
        .pop()
        .expect("one linked entity")
}

#[test]
fn context_and_counter_terms_match_whole_words_and_ground_the_exact_surface() {
    let entry = EntityIndexEntry {
        id: "test:film".into(),
        canonical_name: "Film".into(),
        entity_type: "OBJECT".into(),
        gloss: String::new(),
        negative_constraints: Vec::new(),
        aliases: BTreeMap::new(),
        context_terms: vec!["film".into()],
        counter_terms: vec!["city".into()],
    };
    assert_eq!(context_score("en", &entry, "PERSON", "filmmaker"), 0);
    assert_eq!(context_score("en", &entry, "PERSON", "film"), 3);
    assert_eq!(context_score("en", &entry, "PERSON", "cityscape"), 0);
    assert_eq!(context_score("en", &entry, "PERSON", "city"), -4);
    assert_eq!(
        term_ranges("filmmaker", "film"),
        Vec::<(usize, usize)>::new()
    );
    assert_eq!(term_ranges("at the Film festival", "film"), vec![(7, 11)]);
}

#[test]
fn another_language_alias_matches_the_same_entry() {
    // The index matches normalized aliases across languages, so the same
    // KB id is returned regardless of the mention language.
    let it = link_one(
        "L'Agenzia Spaziale Europea studia Marte.",
        "Agenzia Spaziale Europea",
        "ORG",
    );
    assert_eq!(it.canonical_id.as_deref(), Some("wd:Q42262"));
}

#[test]
fn canonical_surface_falls_back_to_the_grounded_first_mention() {
    // A surname alias can promote a one-token mention to a full name the
    // script never states ("Leggere Augias" for a bare "Augias"). The
    // manifest must publish a grounded surface, otherwise the Go handoff
    // rejects the whole run for an ungrounded canonical mention.
    let identities = vec![CanonicalIdentity {
        id: "person_01".to_string(),
        canonical_id: "person:leggere-augias".to_string(),
        canonical_surface: "Leggere Augias".to_string(),
        kind: "PERSON".to_string(),
        mentions: vec![Mention {
            scene_id: "s1".to_string(),
            surface: "Augias".to_string(),
            start_byte: 0,
            end_byte: 6,
        }],
    }];
    let linked = link_all(
        "it",
        &[("s1", "Augias ha intrecciato una vita di cultura.")],
        &identities,
    );
    assert_eq!(linked[0].canonical_surface, "Augias");
    assert_eq!(
        linked[0].surface_key,
        stable_surface_key("PERSON", "Augias")
    );
}

#[test]
fn ambiguous_alias_without_context_is_not_linked() {
    let nike = link_one("Nike ha annunciato una nuova iniziativa.", "Nike", "BRAND");
    assert_eq!(nike.link_status, LinkStatus::Ambiguous);
    assert!(nike.canonical_id.is_none());
}

#[test]
fn context_resolves_nike_to_sportswear() {
    let nike = link_one(
        "Nike ha presentato le nuove scarpe da running per gli atleti.",
        "Nike",
        "BRAND",
    );
    assert_eq!(nike.link_status, LinkStatus::Linked);
    assert_eq!(nike.canonical_id.as_deref(), Some("wd:Q483859"));
    assert!(!nike.negative_constraints.is_empty());
}

#[test]
fn context_resolves_esa_away_from_video_games() {
    let esa = link_one(
        "ESA e NASA studiano Marte con nuove missioni spaziali.",
        "ESA",
        "ORG",
    );
    assert_eq!(esa.canonical_id.as_deref(), Some("wd:Q42262"));
    assert!(esa
        .negative_constraints
        .iter()
        .any(|constraint| constraint.contains("video game")));
}

#[test]
fn discourse_prior_carries_a_resolved_sense_only_within_its_thread() {
    let first = "Mercury has an orbit around the sun.";
    let second = "Mercury was mentioned again.";
    let identity = CanonicalIdentity {
        id: "loc_mercury".into(),
        canonical_id: "location:mercury".into(),
        canonical_surface: "Mercury".into(),
        kind: "LOCATION".into(),
        mentions: vec![
            Mention {
                scene_id: "s1".into(),
                surface: "Mercury".into(),
                start_byte: 0,
                end_byte: 7,
            },
            Mention {
                scene_id: "s2".into(),
                surface: "Mercury".into(),
                start_byte: 0,
                end_byte: 7,
            },
        ],
    };
    let scenes = [("s1", first), ("s2", second)];
    let thread = crate::narrative::EntityThread {
        entity_id: "loc_mercury".into(),
        canonical_surface: "Mercury".into(),
        entity_type: "LOCATION".into(),
        scene_ids: vec!["s1".into(), "s2".into()],
        first_scene_id: "s1".into(),
        last_scene_id: "s2".into(),
        mention_count: 2,
        return_count: 0,
        beats: Vec::new(),
    };
    let linked = link_all_with_threads(
        "en",
        &scenes,
        std::slice::from_ref(&identity),
        std::slice::from_ref(&thread),
    )
    .pop()
    .unwrap();
    assert_eq!(linked.canonical_id.as_deref(), Some("wd:Q308"));
    let prior = linked.link_candidates[0]
        .features
        .iter()
        .find(|feature| feature.name == "discourse_prior")
        .expect("winning sense should expose its discourse prior");
    assert_eq!(prior.value, link_weights().discourse_prior_weight);
    assert_eq!(prior.terms, ["orbit"]);
    assert_eq!(prior.spans.len(), 1);
    assert_eq!(
        &first[prior.spans[0].start_byte..prior.spans[0].end_byte],
        "orbit"
    );

    let wrong_thread = crate::narrative::EntityThread {
        entity_id: "another_entity".into(),
        ..thread
    };
    let without_prior = link_all_with_threads(
        "en",
        &scenes,
        std::slice::from_ref(&identity),
        std::slice::from_ref(&wrong_thread),
    )
    .pop()
    .unwrap();
    assert!(without_prior.link_candidates[0]
        .features
        .iter()
        .all(|feature| feature.name != "discourse_prior"));

    let wrong_type = crate::narrative::EntityThread {
        entity_type: "PERSON".into(),
        ..crate::narrative::EntityThread {
            entity_id: "loc_mercury".into(),
            canonical_surface: "Mercury".into(),
            entity_type: "LOCATION".into(),
            scene_ids: vec!["s1".into(), "s2".into()],
            first_scene_id: "s1".into(),
            last_scene_id: "s2".into(),
            mention_count: 2,
            return_count: 0,
            beats: Vec::new(),
        }
    };
    let without_cross_type_prior = link_all_with_threads("en", &scenes, &[identity], &[wrong_type])
        .pop()
        .unwrap();
    assert!(without_cross_type_prior.link_candidates[0]
        .features
        .iter()
        .all(|feature| feature.name != "discourse_prior"));
}

#[test]
fn linked_entities_expose_bounded_candidates_and_byte_exact_evidence() {
    let text = "ESA e NASA studiano Marte con nuove missioni spaziali.";
    let entity = link_one(text, "ESA", "ORG");
    assert_eq!(entity.canonical_id.as_deref(), Some("wd:Q42262"));
    assert_eq!(entity.link_rule.as_deref(), Some("entity_linker.v3"));
    assert_eq!(entity.link_candidates.len(), 2);
    assert_eq!(entity.link_candidates[0].kb_id, "wd:Q42262");
    assert!(entity.link_margin.is_some_and(|margin| margin > 0));
    assert!(entity
        .link_candidates
        .iter()
        .all(|candidate| candidate.features.len() <= 6));
    assert!(entity.link_candidates[0]
        .features
        .iter()
        .any(|feature| feature.name == "context_overlap" && !feature.spans.is_empty()));
    for candidate in &entity.link_candidates {
        for feature in &candidate.features {
            for span in &feature.spans {
                let evidence = text
                    .get(span.start_byte..span.end_byte)
                    .expect("evidence span is UTF-8 aligned");
                assert!(feature
                    .terms
                    .iter()
                    .any(|term| evidence.eq_ignore_ascii_case(term)));
            }
        }
    }
}

#[test]
fn entity_type_cues_are_localized_and_keep_exact_evidence_spans() {
    let text = "ESA è un'agenzia spaziale.";
    let identity = identity("org_01", "ESA", "ORG", "s1");
    let terms = matching_type_cues("it", "ORG", text);
    assert!(terms.iter().any(|term| term == "agenzia"));
    let spans = evidence_spans("agenzia", &identity, &[("s1", text)]);
    assert_eq!(spans.len(), 1);
    assert_eq!(&text[spans[0].start_byte..spans[0].end_byte], "agenzia");
    assert!(matching_type_cues("en", "ORG", "ESA è un'agenzia spaziale.").is_empty());
}

#[test]
fn every_confusable_index_sense_has_three_positive_and_negative_contexts() {
    #[derive(Deserialize)]
    struct FixtureFile {
        schema_version: String,
        surfaces: Vec<SurfaceFixture>,
    }
    #[derive(Deserialize)]
    struct SurfaceFixture {
        surface: String,
        senses: Vec<SenseFixture>,
    }
    #[derive(Deserialize)]
    struct SenseFixture {
        id: String,
        entity_type: String,
        positive_contexts: Vec<String>,
        negative_contexts: Vec<NegativeFixture>,
    }
    #[derive(Deserialize)]
    struct NegativeFixture {
        context: String,
        expected_id: String,
    }

    let fixtures: FixtureFile = serde_json::from_str(include_str!(
        "../../lexicon/entity_index_confusables_v1.json"
    ))
    .expect("confusable index fixtures must be valid JSON");
    assert_eq!(fixtures.schema_version, "entity_index_confusables.v1");
    let mut positive_cases = 0;
    let mut negative_cases = 0;

    for surface in &fixtures.surfaces {
        assert!(surface.senses.len() >= 2 && surface.senses.len() <= 3);
        for sense in &surface.senses {
            assert_eq!(
                sense.positive_contexts.len(),
                3,
                "{} / {}",
                surface.surface,
                sense.id
            );
            assert_eq!(
                sense.negative_contexts.len(),
                3,
                "{} / {}",
                surface.surface,
                sense.id
            );
            for context in &sense.positive_contexts {
                let linked = link_one(context, &surface.surface, &sense.entity_type);
                assert_eq!(
                    linked.canonical_id.as_deref(),
                    Some(sense.id.as_str()),
                    "{context}"
                );
                positive_cases += 1;
            }
            for case in &sense.negative_contexts {
                let linked = link_one(&case.context, &surface.surface, &sense.entity_type);
                assert_eq!(
                    linked.canonical_id.as_deref(),
                    Some(case.expected_id.as_str()),
                    "{}",
                    case.context
                );
                negative_cases += 1;
            }
        }
        let first = &surface.senses[0];
        let linked = link_one(&surface.surface, &surface.surface, &first.entity_type);
        assert_eq!(
            linked.link_status,
            LinkStatus::Ambiguous,
            "{} without context",
            surface.surface
        );
    }

    assert!(fixtures.surfaces.len() >= 40);
    assert!(
        positive_cases >= 240,
        "only {positive_cases} positive contexts tested"
    );
    assert!(
        negative_cases >= 240,
        "only {negative_cases} negative contexts tested"
    );
}

#[test]
fn intents_are_identity_first_and_bounded() {
    let jordan = link_one(
        "Michael Jordan ha parlato con la NASA durante una missione.",
        "Michael Jordan",
        "PERSON",
    );
    assert_eq!(jordan.search_intents[0].kind, "identity");
    assert!(jordan.search_intents.len() <= 3);
    assert!(jordan
        .search_intents
        .iter()
        .all(|intent| !intent.query.contains('.')));
}

#[test]
fn contextual_intents_are_anchored_to_the_owner_identity() {
    let owner = CanonicalIdentity {
        id: "person_augias".into(),
        canonical_id: "person:augias".into(),
        canonical_surface: "Corrado Augias".into(),
        kind: "PERSON".into(),
        mentions: vec![Mention {
            scene_id: "s1".into(),
            surface: "Corrado Augias".into(),
            start_byte: 0,
            end_byte: 14,
        }],
    };
    let place = CanonicalIdentity {
        id: "loc_paris".into(),
        canonical_id: "location:paris".into(),
        canonical_surface: "Paris".into(),
        kind: "LOCATION".into(),
        mentions: vec![Mention {
            scene_id: "s1".into(),
            surface: "Paris".into(),
            start_byte: 20,
            end_byte: 25,
        }],
    };
    let intents = build_intents(
        "it",
        &owner,
        LinkStatus::Linked,
        "Corrado Augias",
        &[owner.clone(), place],
    );
    assert!(intents.iter().any(|intent| intent.kind == "contextual"));
    assert!(intents
        .iter()
        .filter(|intent| intent.kind == "contextual")
        .all(|intent| intent.query.contains("Corrado Augias")));
}

#[test]
fn a_named_entity_without_a_usable_identity_query_publishes_no_intents() {
    // A surface longer than the identity window (or one carrying commas)
    // cannot become the identity anchor. Publishing only the contextual
    // co-mention would break the consumer contract, which requires every
    // named entity's first intent to be its identity.
    let owner = CanonicalIdentity {
        id: "person_augias".into(),
        canonical_id: "person:corrado-augias".into(),
        canonical_surface: "Corrado Augias, giornalista e scrittore italiano".into(),
        kind: "PERSON".into(),
        mentions: vec![Mention {
            scene_id: "s1".into(),
            surface: "Corrado Augias, giornalista e scrittore italiano".into(),
            start_byte: 0,
            end_byte: 45,
        }],
    };
    let place = CanonicalIdentity {
        id: "loc_paris".into(),
        canonical_id: "location:paris".into(),
        canonical_surface: "Paris".into(),
        kind: "LOCATION".into(),
        mentions: vec![Mention {
            scene_id: "s1".into(),
            surface: "Paris".into(),
            start_byte: 50,
            end_byte: 55,
        }],
    };
    let intents = build_intents(
        "it",
        &owner,
        LinkStatus::Linked,
        "Corrado Augias, giornalista e scrittore italiano",
        &[owner.clone(), place],
    );
    assert!(
        intents.is_empty(),
        "no identity anchor must produce no intents, got {intents:?}"
    );
}

#[test]
fn a_visual_concept_never_gets_an_identity_intent() {
    let concept = link_one(
        "le scarpe da running",
        "scarpe da running",
        "VISUAL_CONCEPT",
    );
    assert!(concept
        .search_intents
        .iter()
        .all(|intent| intent.kind != "identity"));
    assert_eq!(concept.role, EntityRole::VisualConcept);
}
