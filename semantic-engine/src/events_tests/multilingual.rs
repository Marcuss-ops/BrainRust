use super::*;

#[test]
fn all_core_event_types_have_grounded_english_and_italian_triggers() {
    let cases = [
        ("en", "Maya Chen was born.", "Maya Chen", "PERSON", "birth"),
        ("en", "Maya Chen died.", "Maya Chen", "PERSON", "death"),
        (
            "en",
            "Maya Chen was convicted.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        (
            "en",
            "Maya Chen was appointed.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        (
            "en",
            "Acme Labs was founded.",
            "Acme Labs",
            "ORG",
            "founding",
        ),
        (
            "en",
            "Acme Labs acquired Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "en",
            "Acme Labs launched Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "en",
            "Maya Chen attacked Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "en",
            "Maya Chen defeated Lee.",
            "Maya Chen",
            "PERSON",
            "match_competition_result",
        ),
        (
            "it",
            "Anna Bianchi è nata.",
            "Anna Bianchi",
            "PERSON",
            "birth",
        ),
        (
            "it",
            "Anna Bianchi nacque a Roma.",
            "Anna Bianchi",
            "PERSON",
            "birth",
        ),
        (
            "it",
            "Anna Bianchi è morta.",
            "Anna Bianchi",
            "PERSON",
            "death",
        ),
        (
            "it",
            "Anna Bianchi morì a Roma.",
            "Anna Bianchi",
            "PERSON",
            "death",
        ),
        (
            "it",
            "Anna Bianchi pubblicò un libro.",
            "Anna Bianchi",
            "PERSON",
            "launch_release",
        ),
        (
            "it",
            "Anna Bianchi restituì un premio.",
            "Anna Bianchi",
            "PERSON",
            "award_return",
        ),
        (
            "it",
            "Anna Bianchi è stata condannata.",
            "Anna Bianchi",
            "PERSON",
            "trial_verdict",
        ),
        (
            "it",
            "Anna Bianchi è stata nominata.",
            "Anna Bianchi",
            "PERSON",
            "appointment_resignation",
        ),
        (
            "it",
            "Acme Labs è stata fondata.",
            "Acme Labs",
            "ORG",
            "founding",
        ),
        (
            "it",
            "Acme Labs ha acquisito Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "it",
            "Acme Labs ha lanciato Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "it",
            "Anna Bianchi ha attaccato Marco.",
            "Anna Bianchi",
            "PERSON",
            "attack_battle",
        ),
        (
            "it",
            "Milan ha sconfitto Roma.",
            "Milan",
            "ORG",
            "match_competition_result",
        ),
    ];

    for (index, (language, source, participant, kind, expected_type)) in
        cases.into_iter().enumerate()
    {
        let candidate = candidate(source, participant, "entity_1", kind);
        let frames = extract_with_model(index, "event", source, language, &[candidate]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for {language}: {source}; got {frames:?}"
        );
    }
}

#[test]
fn all_core_event_types_have_grounded_spanish_triggers() {
    let cases = [
        ("Maya Chen nació.", "Maya Chen", "PERSON", "birth"),
        ("Maya Chen murió.", "Maya Chen", "PERSON", "death"),
        ("Maya Chen fue arrestada.", "Maya Chen", "PERSON", "arrest"),
        (
            "Maya Chen fue condenada.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        ("Maya Chen fue elegida.", "Maya Chen", "PERSON", "election"),
        (
            "Maya Chen fue nombrada.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        ("Acme Labs fue fundada.", "Acme Labs", "ORG", "founding"),
        (
            "Acme Labs adquirió Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "Acme Labs lanzó Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "El camión se estrelló.",
            "camión",
            "VEHICLE",
            "accident_disaster",
        ),
        (
            "Maya Chen atacó a Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "Maya Chen derrotó a Lee.",
            "Maya Chen",
            "PERSON",
            "match_competition_result",
        ),
    ];

    for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
        let candidate = candidate(source, participant, "entity_1", kind);
        let frames = extract_with_model(index, "event", source, "es", &[candidate]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for es: {source}; got {frames:?}"
        );
    }
}

#[test]
fn all_core_event_types_have_grounded_french_triggers() {
    let cases = [
        ("Maya Chen est née à Paris.", "Maya Chen", "PERSON", "birth"),
        (
            "Maya Chen est morte à Paris.",
            "Maya Chen",
            "PERSON",
            "death",
        ),
        ("Maya Chen a été arrêtée.", "Maya Chen", "PERSON", "arrest"),
        (
            "Maya Chen a été condamnée.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        ("Maya Chen a été élue.", "Maya Chen", "PERSON", "election"),
        (
            "Maya Chen a été nommée.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        ("Acme Labs a été fondée.", "Acme Labs", "ORG", "founding"),
        (
            "Acme Labs a acquis Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "Acme Labs a lancé Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "Avion Orion s'est écrasé.",
            "Avion Orion",
            "VEHICLE",
            "accident_disaster",
        ),
        (
            "Maya Chen a attaqué Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "Milan a battu Roma.",
            "Milan",
            "ORG",
            "match_competition_result",
        ),
    ];

    for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
        let candidate = candidate(source, participant, "entity_fr", kind);
        let frames = extract(index, "event-fr", source, "fr", &[candidate], &[], &[]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for fr: {source}; got {frames:?}"
        );
    }
}

#[test]
fn french_negation_is_preserved_on_event_frames() {
    let source = "Maya Chen n'a pas été arrêtée.";
    let candidates = vec![candidate(source, "Maya Chen", "per_fr", "PERSON")];
    let frames = extract(0, "s-fr", source, "fr", &candidates, &[], &[]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].status, "negated");
}

#[test]
fn french_uncertainty_is_preserved_on_event_frames() {
    let source = "Maya Chen aurait été arrêtée.";
    let candidates = vec![candidate(source, "Maya Chen", "per_fr", "PERSON")];
    let frames = extract(0, "s-fr", source, "fr", &candidates, &[], &[]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].status, "uncertain");
}

#[test]
fn all_core_event_types_have_grounded_portuguese_triggers() {
    let cases = [
        (
            "Maya Chen nasceu em Lisboa.",
            "Maya Chen",
            "PERSON",
            "birth",
        ),
        (
            "Maya Chen morreu em Lisboa.",
            "Maya Chen",
            "PERSON",
            "death",
        ),
        ("Maya Chen foi presa.", "Maya Chen", "PERSON", "arrest"),
        (
            "Maya Chen foi condenada.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        ("Maya Chen foi eleita.", "Maya Chen", "PERSON", "election"),
        (
            "Maya Chen foi nomeada.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        ("Acme Labs foi fundada.", "Acme Labs", "ORG", "founding"),
        (
            "Acme Labs adquiriu Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "Acme Labs lançou Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "Avião Orion sofreu um acidente.",
            "Avião Orion",
            "VEHICLE",
            "accident_disaster",
        ),
        (
            "Maya Chen atacou Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "Milan venceu Roma.",
            "Milan",
            "ORG",
            "match_competition_result",
        ),
    ];

    for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
        let candidate = candidate(source, participant, "entity_pt", kind);
        let frames = extract(index, "event-pt", source, "pt", &[candidate], &[], &[]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for pt: {source}; got {frames:?}"
        );
    }
}

#[test]
fn portuguese_negation_and_uncertainty_are_preserved_on_event_frames() {
    for (source, expected) in [
        ("Maya Chen não foi presa.", "negated"),
        ("Maya Chen foi presa, supostamente.", "uncertain"),
    ] {
        let candidates = vec![candidate(source, "Maya Chen", "per_pt", "PERSON")];
        let frames = extract(0, "s-pt", source, "pt", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1, "{source}: {frames:?}");
        assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
    }
}

#[test]
fn all_core_event_types_have_grounded_german_triggers() {
    let cases = [
        (
            "Maya Chen wurde in Berlin geboren.",
            "Maya Chen",
            "PERSON",
            "birth",
        ),
        ("Maya Chen starb in Berlin.", "Maya Chen", "PERSON", "death"),
        (
            "Maya Chen wurde festgenommen.",
            "Maya Chen",
            "PERSON",
            "arrest",
        ),
        (
            "Maya Chen wurde verurteilt.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        (
            "Maya Chen wurde gewählt.",
            "Maya Chen",
            "PERSON",
            "election",
        ),
        (
            "Maya Chen wurde ernannt.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        ("Acme Labs wurde gegründet.", "Acme Labs", "ORG", "founding"),
        (
            "Acme Labs übernahm Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "Acme Labs veröffentlichte Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "Flugzeug Orion stürzte ab.",
            "Flugzeug Orion",
            "VEHICLE",
            "accident_disaster",
        ),
        (
            "Maya Chen attackierte Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "Milan besiegte Roma.",
            "Milan",
            "ORG",
            "match_competition_result",
        ),
    ];

    for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
        let candidate = candidate(source, participant, "entity_de", kind);
        let frames = extract(index, "event-de", source, "de", &[candidate], &[], &[]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for de: {source}; got {frames:?}"
        );
    }
}

#[test]
fn german_negation_and_uncertainty_are_preserved_on_event_frames() {
    for (source, expected) in [
        ("Maya Chen wurde nicht festgenommen.", "negated"),
        ("Maya Chen wurde angeblich festgenommen.", "uncertain"),
    ] {
        let candidates = vec![candidate(source, "Maya Chen", "per_de", "PERSON")];
        let frames = extract(0, "s-de", source, "de", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1, "{source}: {frames:?}");
        assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
    }
}

#[test]
fn all_core_event_types_have_grounded_dutch_triggers() {
    let cases = [
        (
            "Maya Chen werd in Amsterdam geboren.",
            "Maya Chen",
            "PERSON",
            "birth",
        ),
        (
            "Maya Chen overleed in Amsterdam.",
            "Maya Chen",
            "PERSON",
            "death",
        ),
        (
            "Maya Chen werd gearresteerd.",
            "Maya Chen",
            "PERSON",
            "arrest",
        ),
        (
            "Maya Chen werd veroordeeld.",
            "Maya Chen",
            "PERSON",
            "trial_verdict",
        ),
        ("Maya Chen werd gekozen.", "Maya Chen", "PERSON", "election"),
        (
            "Maya Chen werd benoemd.",
            "Maya Chen",
            "PERSON",
            "appointment_resignation",
        ),
        ("Acme Labs werd opgericht.", "Acme Labs", "ORG", "founding"),
        (
            "Acme Labs verwierf Beta Corp.",
            "Acme Labs",
            "ORG",
            "acquisition_merger",
        ),
        (
            "Acme Labs publiceerde Orion.",
            "Acme Labs",
            "ORG",
            "launch_release",
        ),
        (
            "Vliegtuig Orion stortte neer.",
            "Vliegtuig Orion",
            "VEHICLE",
            "accident_disaster",
        ),
        (
            "Maya Chen bestormde Lee.",
            "Maya Chen",
            "PERSON",
            "attack_battle",
        ),
        (
            "Milan versloeg Roma.",
            "Milan",
            "ORG",
            "match_competition_result",
        ),
    ];

    for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
        let candidate = candidate(source, participant, "entity_nl", kind);
        let frames = extract(index, "event-nl", source, "nl", &[candidate], &[], &[]);
        assert!(
            frames.iter().any(|frame| frame.event_type == expected_type),
            "missing {expected_type} frame for nl: {source}; got {frames:?}"
        );
    }
}

#[test]
fn dutch_negation_and_uncertainty_are_preserved_on_event_frames() {
    for (source, expected) in [
        ("Maya Chen werd niet gearresteerd.", "negated"),
        ("Maya Chen werd vermoedelijk gearresteerd.", "uncertain"),
    ] {
        let candidates = vec![candidate(source, "Maya Chen", "per_nl", "PERSON")];
        let frames = extract(0, "s-nl", source, "nl", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1, "{source}: {frames:?}");
        assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
    }
}

#[test]
fn all_core_event_types_have_grounded_indonesian_polish_russian_and_turkish_triggers() {
    for language in ["id", "pl", "ru", "tr"] {
        let file = lexicon(language).expect("event lexicon should be compiled");
        for (event_type, triggers) in file.event_types {
            let first = &triggers[0];
            let trigger = first.surface.as_deref().unwrap_or(&first.lemma);
            let source = format!("Maya Chen {trigger}.");
            let candidate = candidate(&source, "Maya Chen", "per_test", "PERSON");
            let frames = extract(0, "event-test", &source, language, &[candidate], &[], &[]);
            assert!(
                frames.iter().any(|frame| frame.event_type == event_type),
                "missing {event_type} frame for {language}: {source}; got {frames:?}"
            );
        }
    }
}

#[test]
fn negation_and_uncertainty_are_preserved_for_new_event_languages() {
    for (language, source, expected) in [
        ("id", "Maya Chen tidak ditangkap.", "negated"),
        ("id", "Maya Chen diduga ditangkap.", "uncertain"),
        ("pl", "Maya Chen nie został aresztowany.", "negated"),
        (
            "pl",
            "Maya Chen został prawdopodobnie aresztowany.",
            "uncertain",
        ),
        ("ru", "Maya Chen не арестован.", "negated"),
        ("ru", "Maya Chen якобы арестован.", "uncertain"),
        ("tr", "Maya Chen tutuklu değil.", "negated"),
        ("tr", "Maya Chen muhtemelen tutuklandı.", "uncertain"),
    ] {
        let candidates = vec![candidate(source, "Maya Chen", "per_test", "PERSON")];
        let frames = extract(0, "event-status", source, language, &candidates, &[], &[]);
        assert_eq!(frames.len(), 1, "{language} {source}: {frames:?}");
        assert_eq!(frames[0].status, expected, "{language} {source}");
    }
}

#[test]
fn spanish_negation_is_preserved_on_event_frames() {
    let source = "Maya Chen no fue arrestada.";
    let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
    let frames = extract_with_model(0, "s1", source, "es", &candidates);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].status, "negated");
}

#[test]
fn frame_cap_keeps_the_earliest_source_events() {
    let source = "Agreement in 2019. Agreement in 2020. Agreement in 2021.";
    let frames = extract_bounded(
        SceneEventInput {
            scene_index: 0,
            scene_id: "s1",
            source,
            language: "en",
            candidates: &[],
            temporal_facts: &semantic_core::temporal_facts(source, "en"),
            places: &[],
        },
        2,
    );
    assert_eq!(frames.len(), 2);
    assert!(frames[0].trigger_start_byte < frames[1].trigger_start_byte);
    assert_eq!(frames[0].event_type, "agreement");
}
