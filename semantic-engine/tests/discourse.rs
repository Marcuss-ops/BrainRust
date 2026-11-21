//! Narrative continuity, coreference and ambiguity contracts.

mod common;

use common::*;
use semantic_core::{DiscourseProjection, SemanticManifest};
use serde_json::Value;

fn discourse_refs(value: &Value) -> &[Value] {
    value["discourse"]["references"]
        .as_array()
        .expect("discourse references array")
}

fn reference<'a>(value: &'a Value, mention: &str) -> &'a Value {
    discourse_refs(value)
        .iter()
        .find(|reference| reference["mention"] == mention)
        .unwrap_or_else(|| panic!("missing reference {mention:?}: {}", value["discourse"]))
}

fn entity_id(value: &Value, surface: &str) -> String {
    value["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|scene| scene["entities"].as_array().unwrap())
        .find(|entity| entity["text"] == surface)
        .unwrap_or_else(|| panic!("missing entity {surface:?}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn explicit_name_returns_after_a_distant_scene_and_keeps_one_thread() {
    let value = manifest_value(vec![
        scene("opening", "Mario Rossi fondò l'azienda nel 2005."),
        scene(
            "middle",
            "La città cambiò quartiere e il comune riorganizzò gli uffici.",
        ),
        scene(
            "finale",
            "Mario Rossi tornò nel 2015 per vendere le sue quote.",
        ),
    ]);
    let mario = entity_id(&value, "Mario Rossi");
    let refs: Vec<&Value> = discourse_refs(&value)
        .iter()
        .filter(|reference| {
            reference["target_entity_ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == mario.as_str()))
        })
        .collect();
    assert!(
        refs.len() >= 2,
        "Mario must have an explicit resolved reference in both scenes: {}",
        value["discourse"]
    );
    let thread = value["discourse"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["entity_id"] == mario.as_str())
        .expect("Mario must have a thread");
    assert_eq!(
        thread["scene_ids"].as_array().unwrap().len(),
        2,
        "the thread must span the two scenes where Mario appears: {thread}"
    );
    assert!(
        thread["return_count"].as_u64().unwrap() >= 1,
        "the return after a skipped scene must be recorded: {thread}"
    );
    assert!(
        thread["beats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|beat| beat["state"] == "returns"),
        "a beat must carry the return state: {thread}"
    );
}

#[test]
fn pronoun_resolves_when_the_script_leaves_one_person() {
    let text = "Mario Rossi fondò un'azienda nel 2005. Lui decise di vendere le sue quote.";
    let value = manifest_value(vec![scene("solo", text)]);
    let mario = entity_id(&value, "Mario Rossi");
    let pronoun = reference(&value, "Lui");
    assert_eq!(pronoun["type"], "pronoun");
    assert_eq!(pronoun["status"], "resolved", "{pronoun}");
    assert_eq!(
        pronoun["target_entity_ids"],
        serde_json::json!([mario]),
        "the pronoun must resolve to the only person: {pronoun}"
    );
    assert_eq!(pronoun["resolution_basis"], "nearest_sentence_antecedent");
}

#[test]
fn pronoun_gender_filters_only_explicitly_gendered_role_antecedents() {
    let value = manifest_value_for(
        "it",
        vec![scene(
            "gender",
            "Anna Bianchi, ministra, ha parlato. Marco Rossi, ministro, ha parlato. Lei ha vinto.",
        )],
    );
    assert_eq!(
        value["analyzers"]["discourse"],
        "discourse.v7.role_nominal_basis"
    );
    let anna = entity_id(&value, "Anna Bianchi");
    let marco = entity_id(&value, "Marco Rossi");
    let anna_role = value["scenes"][0]["person_roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["person_id"] == anna)
        .unwrap();
    let marco_role = value["scenes"][0]["person_roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["person_id"] == marco)
        .unwrap();
    assert_eq!(anna_role["gender"], "feminine");
    assert_eq!(marco_role["gender"], "masculine");
    let lei = reference(&value, "Lei");
    assert_eq!(lei["status"], "resolved");
    assert_eq!(lei["target_entity_ids"][0], anna);
    assert_eq!(lei["resolution_basis"], "nearest_sentence_antecedent");
}

#[test]
fn gendered_role_nominal_filters_only_known_conflicting_candidates() {
    let value = manifest_value_for(
        "it",
        vec![scene(
            "role-gender",
            "Anna Bianchi e Marco Rossi erano presenti. La fondatrice ha parlato.",
        )],
    );
    let anna = entity_id(&value, "Anna Bianchi");
    let founder_reference = reference(&value, "fondatrice");
    assert_eq!(
        founder_reference["status"], "resolved",
        "{founder_reference}"
    );
    assert_eq!(
        founder_reference["target_entity_ids"],
        serde_json::json!([anna])
    );
    assert_eq!(founder_reference["resolution_basis"], "role_nominal");

    let common = manifest_value_for(
        "it",
        vec![scene(
            "role-gender-unknown",
            "Anna Bianchi e Marco Rossi erano presenti. Il presidente ha parlato.",
        )],
    );
    let common_reference = reference(&common, "presidente");
    assert_eq!(
        common_reference["status"], "ambiguous",
        "{common_reference}"
    );
    assert_eq!(
        common_reference["candidate_entity_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn curated_given_name_gender_filters_only_known_conflicts() {
    let value = manifest_value_for(
        "it",
        vec![scene(
            "name-gender",
            "Anna Bianchi ha parlato. Marco Rossi ha parlato. Lei ha vinto.",
        )],
    );
    let anna = entity_id(&value, "Anna Bianchi");
    let lei = reference(&value, "Lei");
    assert_eq!(lei["status"], "resolved");
    assert_eq!(lei["target_entity_ids"], serde_json::json!([anna]));

    let uncertain = manifest_value_for(
        "it",
        vec![scene(
            "unknown-gender",
            "Alex Smith ha incontrato Taylor Jones e Lei ha vinto.",
        )],
    );
    let lei = reference(&uncertain, "Lei");
    assert_eq!(lei["status"], "ambiguous");
    assert_eq!(lei["candidate_entity_ids"].as_array().unwrap().len(), 2);
}

#[test]
fn turkish_genderless_pronoun_resolves_with_one_grounded_person() {
    let value = manifest_value_for(
        "tr",
        vec![scene("turkish", "Ayşe Yılmaz geldi. O gülümsedi.")],
    );
    let person = entity_id(&value, "Ayşe Yılmaz");
    let pronoun = reference(&value, "O");
    assert_eq!(pronoun["status"], "resolved");
    assert_eq!(pronoun["target_entity_ids"], serde_json::json!([person]));
}

#[test]
fn pronoun_stays_ambiguous_when_two_people_are_plausible() {
    let value = manifest_value(vec![scene(
        "due-persone",
        "Mario Rossi incontrò Luca Bianchi. Lui sorrise.",
    )]);
    let pronoun = reference(&value, "Lui");
    assert_eq!(
        pronoun["status"], "ambiguous",
        "two recent people must not be guessed: {pronoun}"
    );
    assert_eq!(
        pronoun["candidate_entity_ids"].as_array().unwrap().len(),
        2,
        "both plausible antecedents must be reported: {pronoun}"
    );
    assert!(
        pronoun["target_entity_ids"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "an ambiguous reference has no target: {pronoun}"
    );
}

#[test]
fn role_nominal_with_two_active_people_is_ambiguous() {
    let text = "Mario Rossi fondò un'azienda nel 2005. \
                Nei primi anni l'impresa crebbe rapidamente. \
                Nel 2010 arrivò un nuovo investitore. \
                L'investitore si chiamava Luca Bianchi. \
                I due entrarono presto in conflitto. \
                Dopo diversi mesi, l'imprenditore decise di vendere le sue quote.";
    let value = manifest_value(vec![scene("continuita", text)]);
    let role = reference(&value, "imprenditore");
    assert_eq!(
        role["status"], "ambiguous",
        "the role must not be assigned to the last name seen: {role}"
    );
    assert_eq!(
        role["candidate_entity_ids"].as_array().unwrap().len(),
        2,
        "both people in the scene are plausible: {role}"
    );
    assert!(role["target_entity_ids"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[test]
fn role_nominal_resolves_when_the_scene_has_one_person() {
    let text = "Mario Rossi fondò l'azienda. L'imprenditore decise di venderla.";
    let value = manifest_value(vec![scene("un-solo", text)]);
    let mario = entity_id(&value, "Mario Rossi");
    let role = reference(&value, "imprenditore");
    assert_eq!(role["status"], "resolved", "{role}");
    assert_eq!(role["target_entity_ids"], serde_json::json!([mario]));
    assert_eq!(role["resolution_basis"], "role_nominal");
}

#[test]
fn plural_phrase_with_exactly_two_people_resolves_both() {
    let value = manifest_value(vec![scene(
        "soci",
        "Mario Rossi e Luca Bianchi fondarono l'azienda. I due decisero di venderla.",
    )]);
    let pronoun = reference(&value, "I due");
    assert_eq!(pronoun["status"], "resolved", "{pronoun}");
    assert_eq!(
        pronoun["target_entity_ids"].as_array().unwrap().len(),
        2,
        "the pair must stay explicit: {pronoun}"
    );
    assert_eq!(pronoun["resolution_basis"], "plural_pair_antecedent");
}

#[test]
fn quoted_pronoun_is_never_attributed_to_the_narrator() {
    let value = manifest_value(vec![scene(
        "citazione",
        "\"Lui è colpevole\", disse Mario Rossi.",
    )]);
    let pronoun = reference(&value, "Lui");
    assert_eq!(pronoun["status"], "unresolved", "{pronoun}");
    assert_eq!(pronoun["reason"], "quoted_speech");
    assert!(pronoun["target_entity_ids"]
        .as_array()
        .is_some_and(Vec::is_empty));
}

#[test]
fn surname_and_full_name_share_one_identity_and_one_thread() {
    let value = manifest_value(vec![
        scene("full", "Alberto Stasi fondò l'azienda."),
        scene("short", "Stasi tornò in tribunale."),
    ]);
    let surname = reference(&value, "Stasi");
    assert_eq!(surname["type"], "surname", "{surname}");
    assert_eq!(surname["status"], "resolved", "{surname}");
    let target = surname["target_entity_ids"][0].as_str().unwrap();
    let thread = value["discourse"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["entity_id"] == target)
        .expect("surname and full name must form one thread");
    assert_eq!(thread["canonical_surface"], "Alberto Stasi");
    assert_eq!(thread["scene_ids"].as_array().unwrap().len(), 2);
}

#[test]
fn negation_does_not_disambiguate_a_pronoun_or_create_a_target() {
    let value = manifest_value(vec![scene(
        "negazione",
        "Mario Rossi non incontrò Luca Bianchi. Lui sorrise.",
    )]);
    let pronoun = reference(&value, "Lui");
    assert_eq!(
        pronoun["status"], "ambiguous",
        "negating the predicate must not choose a referent: {pronoun}"
    );
    assert_eq!(pronoun["candidate_entity_ids"].as_array().unwrap().len(), 2);
    assert!(
        pronoun["target_entity_ids"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "no target may be invented under negation: {pronoun}"
    );
}

#[test]
fn same_surname_does_not_merge_two_people_or_pick_a_pronoun_target() {
    let value = manifest_value(vec![scene(
        "omonimi",
        "Mario Rossi incontrò Luca Rossi. Lui sorrise.",
    )]);
    let persons: Vec<&Value> = value["scenes"][0]["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entity| entity["type"] == "PERSON")
        .collect();
    assert_eq!(
        persons.len(),
        2,
        "two same-surname people must stay distinct"
    );
    assert_ne!(persons[0]["id"], persons[1]["id"]);
    let pronoun = reference(&value, "Lui");
    assert_eq!(pronoun["status"], "ambiguous", "{pronoun}");
    assert_eq!(pronoun["candidate_entity_ids"].as_array().unwrap().len(), 2);
}

#[test]
fn pronoun_after_twenty_sentences_resolves_only_by_global_uniqueness() {
    let mut sentences = vec!["Mario Rossi fondò l'azienda nel 2005.".to_string()];
    for index in 0..20 {
        sentences.push(format!(
            "La città cambiò il quartiere numero {index} e riorganizzò gli uffici."
        ));
    }
    sentences.push("Lui tornò per vendere le sue quote.".to_string());
    let value = manifest_value(vec![scene("lungo", &sentences.join(" "))]);
    let mario = entity_id(&value, "Mario Rossi");
    let pronoun = reference(&value, "Lui");
    assert_eq!(pronoun["status"], "resolved", "{pronoun}");
    assert_eq!(pronoun["target_entity_ids"], serde_json::json!([mario]));
    assert_eq!(
        pronoun["resolution_basis"],
        "globally_unique_compatible_entity"
    );
}

#[test]
fn manifest_from_before_discourse_decodes_with_an_empty_projection() {
    let mut value = manifest_value(vec![scene("legacy", "Mario Rossi è un nome.")]);
    value.as_object_mut().unwrap().remove("discourse");
    let decoded: SemanticManifest = serde_json::from_value(value).expect("legacy manifest");
    assert_eq!(decoded.discourse, DiscourseProjection::default());
    assert_eq!(decoded.discourse.schema, "discourse.v1");
    assert!(decoded.discourse.references.is_empty());
}

#[test]
fn discourse_counts_match_the_reference_statuses() {
    let value = manifest_value(vec![scene(
        "conteggi",
        "Mario Rossi e Luca Bianchi entrarono. Lui sorrise. L'imprenditore attese.",
    )]);
    let refs = discourse_refs(&value);
    let counts = &value["discourse"]["counts"];
    assert_eq!(counts["total"], refs.len());
    assert_eq!(
        counts["resolved"].as_u64().unwrap()
            + counts["ambiguous"].as_u64().unwrap()
            + counts["unresolved"].as_u64().unwrap(),
        refs.len() as u64
    );
    assert!(counts["ambiguous"].as_u64().unwrap() >= 1);
}

#[test]
fn scene_continuity_reports_shared_and_shifted_entity_sets() {
    let value = manifest_value(vec![
        scene("uno", "Mario Rossi fondò l'azienda."),
        scene("due", "Mario Rossi e Luca Bianchi entrarono in conflitto."),
        scene(
            "tre",
            "Il tribunale ascoltò una testimone e rinviò l'udienza.",
        ),
    ]);
    let continuity = value["discourse"]["scene_continuity"].as_array().unwrap();
    assert_eq!(continuity.len(), 3);
    assert_eq!(continuity[0]["status"], "start");
    assert_eq!(continuity[1]["status"], "continues");
    assert!(
        continuity[1]["score"].as_f64().unwrap() > 0.0,
        "the shared Mario identity must produce a positive overlap: {}",
        continuity[1]
    );
    assert_ne!(
        continuity[2]["status"], "continues",
        "a scene with no shared resolved entity must not claim continuity: {}",
        continuity[2]
    );
}

#[test]
fn german_and_dutch_plural_verb_agreement_resolves_number_ambiguous_pronouns() {
    for (language, text, pronoun) in [
        (
            "de",
            "Hans Müller und Thomas Fischer kamen an. Sie feierten.",
            "Sie",
        ),
        (
            "nl",
            "Sem de Boer en Milan Mulder kwamen aan. Zij vierden feest.",
            "Zij",
        ),
    ] {
        let value = manifest_value_for(language, vec![scene("pair", text)]);
        let resolved = reference(&value, pronoun);
        assert_eq!(resolved["status"], "resolved", "{language}: {resolved}");
        assert_eq!(
            resolved["target_entity_ids"].as_array().unwrap().len(),
            2,
            "{language}: plural agreement must retain both antecedents"
        );
        assert_eq!(resolved["resolution_basis"], "agreement");
    }
}

#[test]
fn singular_verb_agreement_does_not_turn_ambiguous_sie_into_a_plural_pair() {
    let value = manifest_value_for(
        "de",
        vec![scene(
            "singular",
            "Anna Schmidt und Maria Weber kamen an. Sie lächelte.",
        )],
    );
    let pronoun = reference(&value, "Sie");
    assert_eq!(pronoun["status"], "ambiguous", "{pronoun}");
    assert_eq!(pronoun["candidate_entity_ids"].as_array().unwrap().len(), 2);
}
