use super::*;
use visualner::ExtractOptions;

fn facts(text: &str) -> (Vec<EntityFact>, Vec<String>) {
    let analysis = visualner::analyze(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 100,
        },
    )
    .expect("analysis");
    let ids: Vec<String> = analysis
        .entities
        .iter()
        .enumerate()
        .map(|(index, entity)| format!("e{}:{}", index, entity.text))
        .collect();
    let sentences = semantic_core::split_sentences(text, "it", None);
    let facts = extract(
        text,
        "it",
        &sentences,
        &analysis.entities,
        &ids,
        &analysis.temporal_facts,
        &analysis.numeric_facts,
    );
    (facts, ids)
}

#[test]
fn acquittal_keeps_the_subject_and_the_same_sentence_date() {
    let (facts, _) = facts("Il 17 dicembre 2009 Alberto Stasi fu assolto.");
    let fact = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
        .unwrap_or_else(|| panic!("acquittal fact: {facts:?}"));
    assert_eq!(fact.predicate, "legal_outcome");
    assert_eq!(fact.status, "asserted");
    assert_eq!(fact.value, "17 dicembre 2009");
    assert!(fact.id.contains("acquitted"));
}

#[test]
fn negated_conviction_is_preserved_as_negated() {
    let (facts, _) = facts("Alberto Stasi non è stato condannato in quel procedimento.");
    let fact = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("not_convicted"))
        .expect("not-convicted fact");
    assert_eq!(fact.status, "asserted");
    assert!(!facts.iter().any(|fact| {
        fact.normalized_value.as_deref() == Some("convicted") && fact.status == "asserted"
    }));
}

#[test]
fn a_second_date_in_the_sentence_does_not_override_the_outcome_date() {
    let (facts, _) = facts(
        "Alberto Stasi fu rinviato a giudizio. Nel 2014 il giudice pronunciò una prima assoluzione, che nel 2016 fu confermata in appello.",
    );
    let acquitted = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
        .expect("acquittal fact");
    assert_eq!(acquitted.normalized_date.as_deref(), Some("2014"));
}

#[test]
fn finality_prefers_the_convicted_person_over_the_court_after_the_marker() {
    let (facts, ids) = facts(
        "Il 17 dicembre 2014 la Corte d'Assise d'Appello di Milano pronunciò una condanna a sedici anni per Alberto Stasi. Il 12 dicembre 2015 la condanna divenne definitiva in Cassazione.",
    );
    let person = ids
        .iter()
        .find(|id| id.contains("Alberto Stasi"))
        .expect("person id");
    let final_fact = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("final"))
        .unwrap_or_else(|| panic!("final fact: {facts:?}"));
    assert_eq!(&final_fact.subject_entity_id, person);
    assert_eq!(final_fact.normalized_date.as_deref(), Some("2015-12-12"));
}

#[test]
fn confirmed_acquittal_does_not_also_emit_a_plain_acquittal() {
    let (facts, _) = facts(
        "Nel 2009 Alberto Stasi fu assolto. Il 6 dicembre 2011 la Corte d'Appello di Milano confermò l'assoluzione.",
    );
    assert!(
        facts.iter().any(|fact| {
            fact.normalized_value.as_deref() == Some("acquittal_confirmed")
                && fact.normalized_date.as_deref() == Some("2011-12-06")
        }),
        "confirmation fact: {facts:?}"
    );
    assert!(
        !facts.iter().any(|fact| {
            fact.normalized_value.as_deref() == Some("acquitted")
                && fact.normalized_date.as_deref() == Some("2011-12-06")
        }),
        "the generic marker duplicated the confirmation: {facts:?}"
    );
}

#[test]
fn final_outcome_is_attributed_to_the_recent_person() {
    let (facts, _) = facts(
        "Nel 2019 Alberto Stasi fu condannato a sedici anni. Nel 2020 questa condanna diventa definitiva.",
    );
    let final_fact = facts
        .iter()
        .find(|fact| fact.predicate == "legal_finality")
        .unwrap_or_else(|| panic!("final fact: {facts:?}"));
    assert_eq!(final_fact.normalized_value.as_deref(), Some("final"));
    assert_eq!(final_fact.normalized_date.as_deref(), Some("2020"));
}

#[test]
fn agentless_outcome_uses_the_only_recent_person() {
    let (facts, ids) = facts(
        "Nel 2009 Alberto Stasi fu assolto. Nel 2014 il giudice pronunciò una prima condanna.",
    );
    let person = ids
        .iter()
        .find(|id| id.contains("Alberto Stasi"))
        .expect("person id");
    let convicted = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("convicted"))
        .unwrap_or_else(|| panic!("convicted fact: {facts:?}"));
    assert_eq!(&convicted.subject_entity_id, person);
    assert_eq!(convicted.normalized_date.as_deref(), Some("2014"));
}

#[test]
fn two_recent_people_disable_the_agentless_fallback() {
    let (facts, _) = facts(
        "Alberto Stasi e Andrea Sempio furono assolti nel 2009. Nel 2014 il giudice pronunciò una condanna.",
    );
    assert!(!facts.iter().any(|fact| {
        fact.normalized_value.as_deref() == Some("convicted")
            && fact.normalized_date.as_deref() == Some("2014")
    }));
}

#[test]
fn negation_never_flips_into_the_opposite_claim() {
    let (facts, _) = facts("Alberto Stasi non fu assolto il 17 dicembre 2009.");
    let fact = facts
        .iter()
        .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
        .expect("negated acquittal is still evidence");
    assert_eq!(fact.status, "negated");
}

#[test]
fn under_investigation_and_denial_stay_distinct() {
    let (facts, _) =
        facts("Andrea Sempio è indagato nel distinto procedimento e respinge le accuse.");
    let statuses: Vec<(&str, &str)> = facts
        .iter()
        .filter(|fact| fact.predicate == "investigation_status")
        .filter_map(|fact| Some((fact.normalized_value.as_deref()?, fact.status.as_str())))
        .collect();
    assert!(statuses.contains(&("under_investigation", "asserted")));
    assert!(statuses.contains(&("denies_accusations", "asserted")));
}

#[test]
fn glossary_definition_does_not_inherit_a_recent_person_as_indagato() {
    let (extracted, _) = facts(
        "Andrea Sempio compare nel racconto. Ogni volta che useremo la parola indagato, intendiamo una persona su cui un procedimento è aperto.",
    );
    assert!(
        extracted
            .iter()
            .all(|fact| fact.predicate != "investigation_status"),
        "a definition is not evidence about the recently named person: {extracted:?}"
    );
}

#[test]
fn a_language_without_a_legal_pack_degrades_instead_of_borrowing_markers() {
    let text = "Il 17 dicembre 2009 Alberto Stasi fu assolto.";
    let analysis = visualner::analyze(
        text,
        &ExtractOptions {
            language: "de".to_string(),
            entity_count: 100,
        },
    )
    .expect("analysis");
    let ids: Vec<String> = analysis
        .entities
        .iter()
        .enumerate()
        .map(|(index, entity)| format!("e{}:{}", index, entity.text))
        .collect();
    let sentences = semantic_core::split_sentences(text, "de", None);
    let facts = extract(
        text,
        "de",
        &sentences,
        &analysis.entities,
        &ids,
        &analysis.temporal_facts,
        &analysis.numeric_facts,
    );
    assert!(
        facts.is_empty(),
        "German has no legal pack, so Italian markers must not fire: {facts:?}"
    );
}

#[test]
fn a_date_backed_fact_requires_the_date_in_the_same_sentence() {
    let (facts, _) = facts("Alberto Stasi fu assolto. Il 17 dicembre 2009 arrivò la decisione.");
    assert!(
        !facts
            .iter()
            .any(|fact| fact.normalized_value.as_deref() == Some("acquitted")),
        "a neighbouring sentence cannot supply the date: {facts:?}"
    );
}
