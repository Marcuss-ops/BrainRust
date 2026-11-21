//! Chart projection, numeric and date-value handling with verbatim evidence.

mod common;

use common::*;

#[test]
fn typed_quantities_carry_scale_unit_and_same_sentence_period() {
    let text = "Il fatturato di Acme è 3,2 miliardi di euro nel 2019.";
    let value = manifest_value(vec![scene("quantita", text)]);
    let quantities = value["scenes"][0]["quantities"]
        .as_array()
        .expect("typed quantities");
    let quantity = quantities.first().expect("revenue amount should be typed");
    assert_eq!(quantity["dimension"], "currency");
    assert_eq!(quantity["unit"], "EUR");
    assert_eq!(quantity["raw_value"], 3.2);
    assert_eq!(quantity["scale"], 1_000_000_000.0);
    assert_eq!(quantity["value"], 3_200_000_000.0);
    assert_eq!(quantity["predicate"], "revenue");
    assert_eq!(quantity["period"]["year"], 2019);
    let start = quantity["start_byte"].as_u64().unwrap() as usize;
    let end = quantity["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&text[start..end], quantity["evidence"]);
}

#[test]
fn chart_bridge_uses_the_grounded_typed_quantity_value() {
    let text = "Il fatturato è cresciuto: 3,2 miliardi di euro nel 2019 contro 1,6 miliardi di euro nel 2018.";
    let value = manifest_value(vec![scene("chart-quantita-typed", text)]);
    let scene = &value["scenes"][0];
    let quantities = scene["quantities"].as_array().expect("quantities");
    let charts = proposals_of(scene, "chart");
    assert_eq!(charts.len(), 1, "one comparable currency chart: {value}");
    let items = charts[0]["chart_items"].as_array().expect("chart items");
    let mut actual: Vec<f64> = items
        .iter()
        .map(|item| item["value"].as_f64().expect("numeric chart value"))
        .collect();
    let mut expected: Vec<f64> = quantities
        .iter()
        .map(|quantity| quantity["value"].as_f64().expect("exact quantity value"))
        .collect();
    actual.sort_by(f64::total_cmp);
    expected.sort_by(f64::total_cmp);
    assert_eq!(actual, expected, "chart values must come from quantities");
    assert_eq!(actual, vec![1_600_000_000.0, 3_200_000_000.0]);
    assert!(items.iter().all(|item| item["kind"] == "MONEY"));
}

#[test]
fn r09_two_percentages_yield_a_chart_with_both_values() {
    let text =
        "Il 42% dei tecnici ha segnalato il problema, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");

    assert_eq!(charts.len(), 1, "one chart expected: {value}");
    let quantities = value["scenes"][0]["quantities"]
        .as_array()
        .expect("typed percent quantities");
    assert_eq!(
        quantities.len(),
        2,
        "both points should be typed: {quantities:?}"
    );
    let mut values: Vec<f64> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![18.0, 42.0]);
    assert_eq!(charts[0]["suggested_visual"], "bar_comparison");
}

#[test]
fn a_comparison_word_inside_a_top_ten_item_does_not_chart_the_ranks() {
    let text = "Nel confronto i risultati sono Aurora 41% contro Lumen 15%.\n\
                Top 10:\n\
                1. documenti originali\n\
                2. rilievi sul campo\n\
                3. confronto dei gruppi\n\
                4. verifica dei costi\n\
                5. cronologia\n\
                6. pareri dei residenti\n\
                7. qualità delle immagini\n\
                8. limiti dei dati\n\
                9. controllo indipendente\n\
                10. pubblicazione dei risultati";
    let value = manifest_value(vec![scene("top-ten-chart-noise", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");

    assert_eq!(
        charts.len(),
        1,
        "only the explicit comparison is a chart: {value}"
    );
    let items = charts[0]["chart_items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        2,
        "rank markers are reference numbers: {items:?}"
    );
    assert!(items.iter().all(|item| item["kind"] == "PERCENT"));
    let mut values: Vec<f64> = items
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![15.0, 41.0]);
}

#[test]
fn top_ten_ranked_quantities_keep_their_ranks_without_becoming_a_chart() {
    let text = "Top 10 revenues:\n\
                1. France earned 10 EUR.\n\
                2. Italy earned 9 EUR.\n\
                3. Spain earned 8 EUR.\n\
                4. Poland earned 7 EUR.\n\
                5. Portugal earned 6 EUR.\n\
                6. Greece earned 5 EUR.\n\
                7. Austria earned 4 EUR.\n\
                8. Belgium earned 3 EUR.\n\
                9. Sweden earned 2 EUR.\n\
                10. Norway earned 1 EUR.";
    let value = manifest_value(vec![scene("ranked-quantities", text)]);
    let scene = &value["scenes"][0];
    let quantities = scene["quantities"].as_array().unwrap();
    assert_eq!(quantities.len(), 10, "typed quantities: {quantities:?}");
    assert!(quantities
        .iter()
        .all(|quantity| { quantity["dimension"] == "currency" && quantity["unit"] == "EUR" }));

    let groups = scene["semantic_groups"].as_array().unwrap();
    let ranking = groups
        .iter()
        .find(|group| group["group_type"] == "ranking")
        .expect("explicit Top 10 ranking");
    assert_eq!(ranking["complete"], true);
    let members = ranking["members"].as_array().unwrap();
    assert_eq!(members.len(), 10);
    for (index, member) in members.iter().enumerate() {
        assert_eq!(member["rank"], index + 1);
    }
    assert!(proposals_of(scene, "chart").is_empty());
}

#[test]
fn spelled_percentages_form_one_chart_of_both_values() {
    // T05: spelled values ("quarantadue per cento") resolve exactly like
    // digits ("42%") when they state their unit.
    let text = "Il quarantadue per cento dei tecnici ha segnalato il problema, \
                contro il diciotto per cento del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto-scritto", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "one chart expected: {value}");
    let mut values: Vec<f64> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![18.0, 42.0]);
}

#[test]
fn a_spelled_value_without_a_unit_enters_no_numeric_projection() {
    // A spelled quantity must state its unit: "quarantadue" alone is not a
    // measurement, so it is neither a chart value nor a numeric fact.
    let text = "Il quarantadue dei tecnici ha segnalato il problema, \
                contro il diciotto del gruppo di controllo.";
    let value = manifest_value(vec![scene("senza-unita", text)]);
    let scene = &value["scenes"][0];
    assert!(proposals_of(scene, "chart").is_empty(), "{value}");
    assert!(
        scene["numeric_facts"].as_array().unwrap().is_empty(),
        "no unit, no numeric fact: {}",
        scene["numeric_facts"]
    );
}

#[test]
fn different_kinds_never_form_a_chart() {
    // Charts compare same-kind values: a percent against money is not a
    // comparison, whatever the comparison cue says.
    let text = "Il 42% dei tecnici ha speso 5.000 euro contro il piano iniziale.";
    let value = manifest_value(vec![scene("tipi-misti", text)]);
    assert!(
        proposals_of(&value["scenes"][0], "chart").is_empty(),
        "{value}"
    );
}

#[test]
fn single_number_never_becomes_a_chart() {
    let text = "Il 42% dei tecnici ha segnalato il problema durante la prova.";
    let value = manifest_value(vec![scene("singolo", text)]);
    assert!(proposals_of(&value["scenes"][0], "chart").is_empty());
}

#[test]
fn unrelated_values_in_one_sentence_do_not_become_a_chart_without_comparison_language() {
    let text = "Il 42% dei clienti ha risposto; il 18% del budget è stato destinato alla ricerca.";
    let value = manifest_value(vec![scene("valori-separati", text)]);
    assert!(proposals_of(&value["scenes"][0], "chart").is_empty());
}

#[test]
fn r08_dates_are_kept_and_no_chart_is_invented() {
    let text = "Il 15 marzo 2008 è la data del delitto. Il 1 giugno 2009 inizia il processo.";
    let value = manifest_value(vec![scene("cronologia", text)]);
    let scene = &value["scenes"][0];

    let facts = scene["temporal_facts"].as_array().unwrap();
    assert!(facts.len() >= 2, "both dates must be kept: {scene}");
    assert!(proposals_of(scene, "chart").is_empty());
}

#[test]
fn chart_items_are_verbatim_source_slices() {
    let text =
        "Il 42% dei tecnici ha segnalato il problema, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let chart = &proposals_of(&value["scenes"][0], "chart")[0];
    for item in chart["chart_items"].as_array().unwrap() {
        let start = item["start_byte"].as_u64().unwrap() as usize;
        let end = item["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), item["text"].as_str());
        assert!(matches!(
            item["kind"].as_str(),
            Some("NUMBER" | "PERCENT" | "MONEY")
        ));
    }
}

#[test]
fn chart_labels_bind_values_to_nearby_subjects_instead_of_copying_the_sentence() {
    let text =
        "Nel 2024 Ford ha venduto 800.000 veicoli e Toyota 1.200.000 veicoli nello stesso mercato.";
    let value = manifest_value(vec![scene("etichette-chart", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "one comparison expected: {value}");
    let mut labels: Vec<&str> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect();
    labels.sort_unstable();
    assert_eq!(labels, ["Ford", "Toyota"], "manifest: {value}");
}

#[test]
fn percentage_labels_keep_the_attached_population_or_group() {
    let text =
        "Il 42% dei tecnici ha segnalato il problema, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("etichette-percentuali", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "one comparison expected: {value}");
    let labels: Vec<&str> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["dei tecnici", "del gruppo di controllo"]);
}

#[test]
fn a_debunked_claim_does_not_suppress_a_separate_true_series() {
    let text = "È falso che Alfa abbia venduto 500 auto. Le vendite effettive sono state 700 nel 2023 e 900 nel 2024.";
    let value = manifest_value(vec![scene("negazione-locale", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    let actual: Vec<f64> = charts
        .iter()
        .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
        .filter_map(|item| item["value"].as_f64())
        .collect();
    assert_eq!(
        actual,
        [700.0, 900.0],
        "only affirmed data is charted: {value}"
    );
    assert!(charts
        .iter()
        .all(|chart| !chart["evidence"].as_str().unwrap().contains("500")));
}

#[test]
fn denied_debunked_and_unverified_numeric_claims_do_not_make_charts() {
    for (id, text) in [
        (
            "negated",
            "Non è vero che il tasso sia passato dal 10% al 30%: quei numeri erano ipotesi smentite.",
        ),
        (
            "debunked",
            "Alfa non ha venduto 500 auto contro le 300 di Beta: la notizia è falsa.",
        ),
        (
            "unverified",
            "Il rapporto cita la frase «nel 2022 la società valeva 10 milioni e nel 2023 20 milioni», ma precisa che questi valori non sono verificati.",
        ),
    ] {
        let value = manifest_value(vec![scene(id, text)]);
        assert!(
            proposals_of(&value["scenes"][0], "chart").is_empty(),
            "a denied, debunked, or unverified claim must not become a chart: {value}"
        );
    }
}

#[test]
fn a_lone_dated_legal_fact_gets_a_date_card() {
    // "emise la sua decisione" is an event the predicate list does not name
    // (it is tuned for construction and outcome stems), but the sentence does
    // assert a dated fact. Without the card the fact had no representation.
    let text = "È fondamentale ricordare la data del 17 dicembre 2009, giorno in cui il Giudice dell'Udienza Preliminare emise la sua decisione di assoluzione nei confronti di Alberto Stasi.";
    let value = manifest_value(vec![scene("gup", text)]);
    let timelines = proposals_of(&value["scenes"][0], "timeline");
    assert_eq!(
        timelines.len(),
        1,
        "one dated event, one card: {timelines:?}"
    );
    assert_eq!(timelines[0]["suggested_visual"], "date_card");
    assert_eq!(
        value["visual_coverage"]["covered_facts"], value["visual_coverage"]["total_facts"],
        "the card represents the dated fact: {}",
        value["visual_coverage"]
    );
    for fact in value["visual_coverage"]["scenes"][0]["facts"]
        .as_array()
        .unwrap()
    {
        assert_eq!(fact["status"], "covered", "every dated fact is covered");
    }
}

#[test]
fn a_date_without_an_asserted_fact_gets_no_card() {
    let text = "Il 17 dicembre 2009 il cielo sopra il paese era limpido.";
    let value = manifest_value(vec![scene("cielo", text)]);
    let timelines = proposals_of(&value["scenes"][0], "timeline");
    assert!(
        timelines.is_empty(),
        "a bare date is not an event: {timelines:?}"
    );
    assert_eq!(value["visual_coverage"]["total_facts"], 0);
}
