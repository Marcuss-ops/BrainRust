use super::*;
use std::collections::BTreeMap;

#[test]
fn event_matching_uses_shared_udpipe_lemma_and_offsets() {
    let source = "Augias nacque a Roma.";
    let start = source.find("nacque").unwrap();
    let trigger = TriggerEntry {
        surface: Some("è nato".into()),
        lemma: "nascere".into(),
        voice: "active".into(),
        roles: BTreeMap::new(),
        pro_drop_roles: Vec::new(),
    };
    let tokens = vec![visualner::AnalyzedToken {
        text: "nacque".into(),
        lemma: Some("nascere".into()),
        upos: Some("VERB".into()),
        features: None,
        dependency: Some("root".into()),
        id: Some(2),
        head: Some(0),
        sentence_id: Some(0),
        start,
        end: start + "nacque".len(),
    }];
    assert_eq!(
        trigger_spans(source, 0, &trigger, &tokens),
        vec![(start, start + "nacque".len())]
    );
}

#[test]
fn event_participant_uses_udpipe_subject_dependency() {
    let source = "Anna Bianchi nacque a Roma.";
    let event_start = source.find("nacque").unwrap();
    let person = candidate(source, "Anna Bianchi", "person_1", "PERSON");
    let tokens = vec![
        visualner::AnalyzedToken {
            text: "Anna".into(),
            lemma: Some("Anna".into()),
            upos: Some("PROPN".into()),
            features: None,
            dependency: Some("nsubj".into()),
            id: Some(1),
            head: Some(3),
            sentence_id: Some(0),
            start: 0,
            end: 4,
        },
        visualner::AnalyzedToken {
            text: "Bianchi".into(),
            lemma: Some("Bianchi".into()),
            upos: Some("PROPN".into()),
            features: None,
            dependency: Some("flat:name".into()),
            id: Some(2),
            head: Some(1),
            sentence_id: Some(0),
            start: 5,
            end: 12,
        },
        visualner::AnalyzedToken {
            text: "nacque".into(),
            lemma: Some("nascere".into()),
            upos: Some("VERB".into()),
            features: Some("VerbForm=Fin".into()),
            dependency: Some("root".into()),
            id: Some(3),
            head: Some(0),
            sentence_id: Some(0),
            start: event_start,
            end: event_start + 6,
        },
    ];
    let sentences = semantic_core::split_sentences(source, "it", None);
    let candidates = [person];
    let found = nearest_candidate(
        &candidates,
        &tokens,
        event_start,
        event_start + 6,
        &sentences[0],
        "subject",
        false,
    );
    assert_eq!(found.map(|value| value.id.as_str()), Some("person_1"));
}

#[test]
fn passive_event_links_the_agent_from_udpipe_oblique_agent() {
    let source = "La Repubblica fu fondata da Eugenio Scalfari.";
    let trigger_start = source.find("fondata").unwrap();
    let organization = candidate(source, "Repubblica", "org_1", "ORGANIZATION");
    let person = candidate(source, "Eugenio Scalfari", "person_1", "PERSON");
    let tokens = vec![
        visualner::AnalyzedToken {
            text: "Repubblica".into(),
            lemma: Some("Repubblica".into()),
            upos: Some("PROPN".into()),
            features: None,
            dependency: Some("nsubj:pass".into()),
            id: Some(2),
            head: Some(4),
            sentence_id: Some(0),
            start: source.find("Repubblica").unwrap(),
            end: source.find("Repubblica").unwrap() + "Repubblica".len(),
        },
        visualner::AnalyzedToken {
            text: "fondata".into(),
            lemma: Some("fondare".into()),
            upos: Some("VERB".into()),
            features: Some("Voice=Pass".into()),
            dependency: Some("root".into()),
            id: Some(4),
            head: Some(0),
            sentence_id: Some(0),
            start: trigger_start,
            end: trigger_start + "fondata".len(),
        },
        visualner::AnalyzedToken {
            text: "Scalfari".into(),
            lemma: Some("Scalfari".into()),
            upos: Some("PROPN".into()),
            features: None,
            dependency: Some("obl:agent".into()),
            id: Some(8),
            head: Some(4),
            sentence_id: Some(0),
            start: source.find("Scalfari").unwrap(),
            end: source.find("Scalfari").unwrap() + "Scalfari".len(),
        },
    ];
    let sentences = semantic_core::split_sentences(source, "it", None);
    let candidates = [organization, person];
    let patient = nearest_candidate(
        &candidates,
        &tokens,
        trigger_start,
        trigger_start + "fondata".len(),
        &sentences[0],
        "subject",
        false,
    );
    let agent = nearest_candidate(
        &candidates,
        &tokens,
        trigger_start,
        trigger_start + "fondata".len(),
        &sentences[0],
        "agent",
        false,
    );
    assert_eq!(patient.map(|value| value.id.as_str()), Some("org_1"));
    assert_eq!(agent.map(|value| value.id.as_str()), Some("person_1"));
}

fn event(scene: &str, date: &str, person: &str) -> EventFrame {
    EventFrame {
        id: format!("ev_{scene}"),
        event_type: "arrest".to_string(),
        trigger: "arrested".to_string(),
        trigger_start_byte: 0,
        trigger_end_byte: 8,
        participants: vec![EventParticipant {
            role: "patient".to_string(),
            entity_id: person.to_string(),
        }],
        place_id: None,
        time: Some(EventTime {
            fact_index: 0,
            normalized_date: Some(date.to_string()),
            derived: false,
        }),
        status: "asserted".to_string(),
        scene_ids: vec![scene.to_string()],
        evidence: vec![EventEvidence {
            scene_id: scene.to_string(),
            start_byte: 0,
            end_byte: 20,
            text: "Maya was arrested.".to_string(),
        }],
        confidence_basis: vec!["trigger".to_string(), "participant".to_string()],
        source_fact_ids: vec![format!("fact_{scene}")],
    }
}

fn thread(entity_id: &str, scene_ids: &[&str]) -> semantic_core::EntityThread {
    semantic_core::EntityThread {
        entity_id: entity_id.to_string(),
        canonical_surface: "Maya Chen".to_string(),
        entity_type: "PERSON".to_string(),
        scene_ids: scene_ids.iter().map(|scene| (*scene).to_string()).collect(),
        first_scene_id: scene_ids[0].to_string(),
        last_scene_id: scene_ids[scene_ids.len() - 1].to_string(),
        mention_count: scene_ids.len(),
        return_count: 0,
        beats: Vec::new(),
    }
}

#[test]
fn cross_scene_events_merge_with_shared_identity_and_date_only() {
    let frames = vec![
        event("s1", "2019", "per_maya"),
        event("s2", "2019", "per_maya"),
    ];
    let no_thread = merge_cross_scene(frames.clone(), &[]);
    assert_eq!(
        no_thread.len(),
        2,
        "identity reuse without a discourse thread must not merge"
    );
    let merged = merge_cross_scene(frames, &[thread("per_maya", &["s1", "s2"])]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].scene_ids, ["s1", "s2"]);
    assert_eq!(merged[0].evidence.len(), 2);
    assert_eq!(merged[0].source_fact_ids, ["fact_s1", "fact_s2"]);

    let threads = [
        thread("per_maya", &["s1", "s2"]),
        thread("per_other", &["s1", "s2"]),
    ];
    let conflicting_dates = merge_cross_scene(
        vec![
            event("s1", "2019", "per_maya"),
            event("s2", "2020", "per_maya"),
        ],
        &threads,
    );
    assert_eq!(conflicting_dates.len(), 2);
    let different_person = merge_cross_scene(
        vec![
            event("s1", "2019", "per_maya"),
            event("s2", "2019", "per_other"),
        ],
        &threads,
    );
    assert_eq!(different_person.len(), 2);
}

fn candidate(source: &str, text: &str, id: &str, kind: &str) -> Candidate {
    let start_byte = source.find(text).unwrap();
    Candidate {
        id: id.to_string(),
        text: text.to_string(),
        kind: kind.to_string(),
        start_byte,
        end_byte: start_byte + text.len(),
        sentence_index: 0,
        evidence: text.to_string(),
        score: 0.9,
        provenance: "test".to_string(),
    }
}

fn extract_with_model(
    scene_index: usize,
    scene_id: &str,
    source: &str,
    language: &str,
    candidates: &[Candidate],
) -> Vec<EventFrame> {
    let analysis = visualner::analyze(
        source,
        &visualner::ExtractOptions {
            language: language.to_string(),
            entity_count: 50,
        },
    )
    .expect("tagged source analysis");
    let temporal = semantic_core::temporal_facts(source, language);
    extract_bounded_with_tokens(
        SceneEventInput {
            scene_index,
            scene_id,
            source,
            language,
            candidates,
            temporal_facts: &temporal,
            places: &[],
        },
        &analysis.tokens,
        MAX_EVENT_FRAMES,
    )
}

fn place(source: &str, text: &str) -> Place {
    let start_byte = source.find(text).unwrap();
    Place {
        id: "pl_1".to_string(),
        entity_local_id: "loc_1".to_string(),
        kb_id: Some("wd:Q90".to_string()),
        link_status: "linked".to_string(),
        kind: "city".to_string(),
        name: text.to_string(),
        lat: Some(48.8566),
        lon: Some(2.3522),
        parent_ids: Vec::new(),
        role: "setting".to_string(),
        start_byte,
        end_byte: start_byte + text.len(),
        sentence_index: 0,
        evidence: text.to_string(),
    }
}

#[test]
fn passive_arrest_frame_attaches_patient_place_and_date() {
    let source = "Maya Chen was arrested in Paris in 2019.";
    let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
    let dates = semantic_core::temporal_facts(source, "en");
    let places = vec![place(source, "Paris")];
    let frames = extract(0, "s1", source, "en", &candidates, &dates, &places);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event_type, "arrest");
    assert_eq!(frames[0].participants[0].role, "patient");
    assert_eq!(frames[0].participants[0].entity_id, "per_1");
    assert_eq!(frames[0].place_id.as_deref(), Some("pl_1"));
    assert_eq!(
        frames[0]
            .time
            .as_ref()
            .and_then(|time| time.normalized_date.as_deref()),
        Some("2019")
    );
    assert_eq!(
        &source[frames[0].trigger_start_byte..frames[0].trigger_end_byte],
        "arrested"
    );
}

#[test]
fn active_arrest_assigns_agent_and_patient_by_grounded_order() {
    let source = "Officer Lee arrested Maya Chen.";
    let candidates = vec![
        candidate(source, "Officer Lee", "per_1", "PERSON"),
        candidate(source, "Maya Chen", "per_2", "PERSON"),
    ];
    let frames = extract(0, "s1", source, "en", &candidates, &[], &[]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].participants.len(), 2);
    assert!(frames[0]
        .participants
        .iter()
        .any(|p| p.role == "agent" && p.entity_id == "per_1"));
    assert!(frames[0]
        .participants
        .iter()
        .any(|p| p.role == "patient" && p.entity_id == "per_2"));
}

#[test]
fn trigger_without_another_grounded_slot_is_dropped() {
    let source = "The arrest was discussed.";
    assert!(extract(0, "s1", source, "en", &[], &[], &[]).is_empty());
}

#[test]
fn negation_is_preserved_on_a_grounded_frame() {
    let source = "Maya Chen was not arrested.";
    let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
    let frames = extract(0, "s1", source, "en", &candidates, &[], &[]);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].status, "negated");
}

#[test]
fn italian_passive_trigger_uses_italian_voice_and_cues() {
    let source = "Anna Bianchi è stata arrestata a Parigi.";
    let candidates = vec![candidate(source, "Anna Bianchi", "per_1", "PERSON")];
    let frames = extract_with_model(0, "s1", source, "it", &candidates);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event_type, "arrest");
    assert_eq!(frames[0].participants[0].role, "patient");
}

#[test]
fn italian_pro_drop_subject_uses_a_unique_recurrent_person_focus() {
    let source = "Corrado Augias lavorò a Roma. Nel 2006 pubblicò un libro.";
    let mut person = candidate(source, "Corrado Augias", "person_corrado", "PERSON");
    let second_mention = Candidate {
        start_byte: source.rfind("Corrado Augias").unwrap(),
        end_byte: source.rfind("Corrado Augias").unwrap() + "Corrado Augias".len(),
        ..person.clone()
    };
    person.sentence_index = 0;
    let frames = extract_with_model(0, "s1", source, "it", &[person, second_mention]);
    let publication = frames
        .iter()
        .find(|frame| frame.event_type == "launch_release")
        .expect("lemma-triggered publication frame");
    assert!(publication.participants.iter().any(|participant| {
        participant.role == "agent" && participant.entity_id == "person_corrado"
    }));
    assert!(publication
        .confidence_basis
        .iter()
        .any(|basis| basis == "pro_drop_coref"));
}

#[test]
fn italian_passive_publication_uses_possessive_for_unique_author_coreference() {
    let source =
            "Corrado Augias fu autore. Tra i suoi successi troviamo Quel treno da Vienna, pubblicato nel 1981.";
    let person = candidate(source, "Corrado Augias", "person_corrado", "PERSON");
    let work = candidate(source, "Quel treno da Vienna", "work_train", "WORK");
    let trigger_start = source.find("pubblicato").unwrap();
    let head_start = source.find("treno").unwrap();
    let tokens = vec![
        visualner::AnalyzedToken {
            text: "treno".into(),
            lemma: Some("treno".into()),
            upos: Some("NOUN".into()),
            features: None,
            dependency: Some("nsubj:pass".into()),
            id: Some(6),
            head: Some(7),
            sentence_id: Some(1),
            start: head_start,
            end: head_start + "treno".len(),
        },
        visualner::AnalyzedToken {
            text: "pubblicato".into(),
            lemma: Some("pubblicare".into()),
            upos: Some("VERB".into()),
            features: Some("VerbForm=Part|Voice=Pass".into()),
            dependency: Some("acl".into()),
            id: Some(7),
            head: Some(6),
            sentence_id: Some(1),
            start: trigger_start,
            end: trigger_start + "pubblicato".len(),
        },
    ];
    let temporal = semantic_core::temporal_facts(source, "it");
    let frames = extract_bounded_with_tokens(
        SceneEventInput {
            scene_index: 0,
            scene_id: "s1",
            source,
            language: "it",
            candidates: &[person.clone(), work.clone()],
            temporal_facts: &temporal,
            places: &[],
        },
        &tokens,
        MAX_EVENT_FRAMES,
    );
    let publication = frames
        .iter()
        .find(|frame| frame.event_type == "launch_release")
        .expect("passive publication frame");
    assert!(publication.participants.iter().any(|participant| {
        participant.role == "agent" && participant.entity_id == "person_corrado"
    }));
    assert!(publication.participants.iter().any(|participant| {
        participant.role == "patient" && participant.entity_id == "work_train"
    }));

    let no_possessive = source.replace("i suoi successi", "i successi");
    let mut person_without_possessive = person;
    person_without_possessive.start_byte = no_possessive.find("Corrado Augias").unwrap();
    person_without_possessive.end_byte =
        person_without_possessive.start_byte + "Corrado Augias".len();
    let work_start = no_possessive.find("Quel treno da Vienna").unwrap();
    let work_without_possessive = Candidate {
        start_byte: work_start,
        end_byte: work_start + "Quel treno da Vienna".len(),
        text: "Quel treno da Vienna".into(),
        evidence: "Quel treno da Vienna".into(),
        ..work
    };
    let trigger_start = no_possessive.find("pubblicato").unwrap();
    let head_start = no_possessive.find("treno").unwrap();
    let no_possessive_tokens = [
        visualner::AnalyzedToken {
            start: head_start,
            end: head_start + "treno".len(),
            ..tokens[0].clone()
        },
        visualner::AnalyzedToken {
            start: trigger_start,
            end: trigger_start + "pubblicato".len(),
            ..tokens[1].clone()
        },
    ];
    let temporal = semantic_core::temporal_facts(&no_possessive, "it");
    let frames = extract_bounded_with_tokens(
        SceneEventInput {
            scene_index: 0,
            scene_id: "s2",
            source: &no_possessive,
            language: "it",
            candidates: &[person_without_possessive, work_without_possessive],
            temporal_facts: &temporal,
            places: &[],
        },
        &no_possessive_tokens,
        MAX_EVENT_FRAMES,
    );
    let publication = frames
        .iter()
        .find(|frame| frame.event_type == "launch_release")
        .expect("passive publication still has the grounded work and date");
    assert!(!publication
        .participants
        .iter()
        .any(|participant| participant.role == "agent"));
}

// A biography often describes the family between two publications, so the
// sentence before the omitted subject names two people. The publication
// history of the scene still identifies the author.
#[test]
fn ambiguous_omitted_subject_reuses_the_scene_established_publication_agent() {
    let source = "Corrado Augias nacque a Roma. Nel 2006 pubblicò Inchiesta su Gesù. Era sposato con Daniela Pasti e aveva una figlia, Natalia. Nel 2024 pubblicò La vita s'impara.";
    let candidates = vec![
        candidate(source, "Corrado Augias", "person_corrado", "PERSON"),
        candidate(source, "Daniela Pasti", "person_daniela", "PERSON"),
        candidate(source, "Natalia", "person_natalia", "PERSON"),
        candidate(source, "Inchiesta su Gesù", "work_gesu", "WORK"),
        candidate(source, "La vita s'impara", "work_vita", "WORK"),
    ];
    let frames = extract_with_model(0, "s1", source, "it", &candidates);
    let publication = frames
        .iter()
        .find(|frame| {
            frame.event_type == "launch_release"
                && frame
                    .evidence
                    .iter()
                    .any(|evidence| evidence.text.contains("2024"))
        })
        .expect("2024 publication frame");
    assert!(publication.participants.iter().any(|participant| {
        participant.role == "agent" && participant.entity_id == "person_corrado"
    }));
    assert!(publication
        .confidence_basis
        .iter()
        .any(|basis| basis == "discourse_agent_coref"));
}

#[test]
fn election_agreement_and_accident_types_have_grounded_slots() {
    let election = "Maya Chen was elected in 2019.";
    let election_candidates = vec![candidate(election, "Maya Chen", "per_1", "PERSON")];
    let election_frames = extract(
        0,
        "s1",
        election,
        "en",
        &election_candidates,
        &semantic_core::temporal_facts(election, "en"),
        &[],
    );
    assert_eq!(election_frames[0].event_type, "election");

    let agreement = "Maya Chen signed an agreement in 2019.";
    let agreement_candidates = vec![candidate(agreement, "Maya Chen", "per_1", "PERSON")];
    let agreement_frames = extract(
        0,
        "s1",
        agreement,
        "en",
        &agreement_candidates,
        &semantic_core::temporal_facts(agreement, "en"),
        &[],
    );
    assert_eq!(agreement_frames[0].event_type, "agreement");
    assert_eq!(agreement_frames[0].participants[0].role, "signatory");

    let accident = "A truck crashed in Paris in 2019.";
    let accident_candidates = vec![candidate(accident, "truck", "veh_1", "VEHICLE")];
    let accident_frames = extract(
        0,
        "s1",
        accident,
        "en",
        &accident_candidates,
        &semantic_core::temporal_facts(accident, "en"),
        &[],
    );
    assert_eq!(accident_frames[0].event_type, "accident_disaster");
    assert_eq!(accident_frames[0].participants[0].role, "vehicle");
}

#[path = "events_tests/multilingual.rs"]
mod multilingual;
