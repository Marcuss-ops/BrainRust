//! Chronology, process phases, completion sequences and timeline replay.

mod common;

use common::*;
use serde_json::Value;

#[test]
fn four_grounded_stadium_phases_form_one_chronological_group() {
    let text = "Il progetto dello stadio fu progettato nel 1950. La costruzione iniziò nel 1952. L'impianto fu inaugurato nel 1959. Lo stadio venne ampliato nel 1974.";
    let value = manifest_value(vec![scene("stadio-aurora", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "one grounded sequence expected: {value}");
    assert_eq!(groups[0]["group_type"], "chronological");
    let members = groups[0]["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        4,
        "all four source phases must be linked: {members:?}"
    );
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(member["temporal_surface"].as_str().is_some());
    }
}

#[test]
fn explicit_process_phases_link_only_when_the_same_entity_is_grounded() {
    let text = "Lo Stadio Aurora fu progettato. La costruzione dello Stadio Aurora iniziò. Lo Stadio Aurora fu inaugurato. Lo Stadio Aurora venne ampliato.";
    let value = manifest_value(vec![scene("processo-stadio", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let process: Vec<_> = groups
        .iter()
        .filter(|group| group["group_type"] == "process")
        .collect();
    assert_eq!(process.len(), 1, "one grounded process expected: {value}");
    let members = process[0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 4, "all explicit phases must be retained");
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(member["evidence"]
            .as_str()
            .unwrap()
            .contains("Stadio Aurora"));
    }
}

#[test]
fn uncertain_or_negated_phases_do_not_complete_a_process_group() {
    let text = "Lo Stadio Aurora fu progettato. La costruzione dello Stadio Aurora non iniziò. Lo Stadio Aurora fu inaugurato. Forse lo Stadio Aurora venne ampliato.";
    let value = manifest_value(vec![scene("processo-incerto", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "process"));
}

#[test]
fn out_of_order_process_phases_are_not_joined() {
    let text = "Lo Stadio Aurora fu progettato. Lo Stadio Aurora venne ampliato. La costruzione dello Stadio Aurora iniziò. Lo Stadio Aurora fu inaugurato.";
    let value = manifest_value(vec![scene("processo-fuori-ordine", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "process"));
}

#[test]
fn process_phases_link_across_scenes_with_local_grounding() {
    let inputs = vec![
        scene("planning", "Lo Stadio Aurora fu progettato."),
        scene("construction", "La costruzione dello Stadio Aurora iniziò."),
        scene("opening", "Lo Stadio Aurora fu inaugurato."),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let process = groups
        .iter()
        .find(|group| group["group_type"] == "process")
        .expect("one cross-scene process should be attached to its first scene");
    let members = process["members"].as_array().unwrap();
    assert_eq!(members.len(), 3);
    for (member, input) in members.iter().zip(inputs) {
        assert_eq!(member["scene_id"], input.scene_id);
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(input.text.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn process_phases_of_different_entities_are_not_joined_across_scenes() {
    let inputs = vec![
        scene("planning", "Lo Stadio Aurora fu progettato."),
        scene("construction", "La costruzione del Ponte Verdi iniziò."),
        scene("opening", "Il Teatro Nuovo fu inaugurato."),
    ];
    let value = manifest_value(inputs);
    for scene in value["scenes"].as_array().unwrap() {
        assert!(scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "process"));
    }
}

#[test]
fn completion_phase_completes_a_four_step_construction_sequence() {
    let text = "La prima fase riguarda la progettazione dello Stadio Aurora. La seconda fase avvia la costruzione dello Stadio Aurora. La terza fase completa le tribune dello Stadio Aurora. La quarta fase amplia la capienza dello Stadio Aurora.";
    let value = manifest_value(vec![scene("quattro-fasi", text)]);
    let process: Vec<&Value> = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| group["group_type"] == "process")
        .collect();
    assert_eq!(process.len(), 1, "one process group expected: {value}");
    let members = process[0]["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        4,
        "all four phases must be linked: {members:?}"
    );
    for (index, member) in members.iter().enumerate() {
        assert_eq!(member["sentence_index"], index);
        assert!(member["evidence"]
            .as_str()
            .unwrap()
            .contains("Stadio Aurora"));
    }
}

#[test]
fn corrected_chronology_yields_the_documented_sequence() {
    // The same passage as the 3,127-word Garlasco fixture, with the trial years
    // corrected to the documented ones (2009, 2011, 2013, 2014, 2015).
    let text = "Passano gli anni e il caso prende una direzione precisa. Alberto Stasi viene rinviato a giudizio e il suo processo si sviluppa in un percorso che durerà diversi anni. Nel duemilanove il giudice di primo grado pronuncia una prima assoluzione, che nel duemilaundici viene confermata nel giudizio d'appello. Per molti sembra la fine di una vicenda, ma non è così. Nel duemilatredici la Cassazione interviene annullando con rinvio la sentenza, e il caso torna in aula con un nuovo giudizio d'appello che si conclude nel duemilaquattordici con una condanna a sedici anni di reclusione. Nel duemilaquindici questa condanna diventa definitiva.";
    let value = manifest_value(vec![scene("processo", text)]);
    let scene_manifest = &value["scenes"][0];

    let mut facts: Vec<(String, String)> = scene_manifest["entity_facts"]
        .as_array()
        .expect("facts array")
        .iter()
        .filter_map(|fact| {
            Some((
                fact["normalized_date"].as_str()?.to_string(),
                fact["normalized_value"].as_str()?.to_string(),
            ))
        })
        .collect();
    facts.sort();
    assert_eq!(
        facts,
        vec![
            ("2009".to_string(), "acquitted".to_string()),
            ("2011".to_string(), "acquittal_confirmed".to_string()),
            ("2013".to_string(), "annulled_with_remand".to_string()),
            ("2014".to_string(), "convicted".to_string()),
            ("2015".to_string(), "final".to_string()),
        ],
        "the engine must reproduce the script's own timeline, not a guessed one"
    );

    let group = scene_manifest["semantic_groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .find(|group| group["group_type"] == "step_sequence")
        .expect("five dated outcomes form one ordered sequence");
    let members = group["members"].as_array().expect("members array");
    let dates: Vec<&str> = members
        .iter()
        .map(|member| member["normalized_date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2009", "2011", "2013", "2014", "2015"]);
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            text.get(start..end),
            member["evidence"].as_str(),
            "every step keeps its verbatim source sentence"
        );
    }
    assert!(
        members[0]["text"]
            .as_str()
            .unwrap()
            .contains("assoluzione nel 2009"),
        "the first step is the 2009 acquittal: {members:?}"
    );
    assert!(
        members[4]["text"]
            .as_str()
            .unwrap()
            .contains("definitiva nel 2015"),
        "the last step is the 2015 finality: {members:?}"
    );
}

#[test]
fn a_complete_dated_sequence_is_one_timeline_and_closes_coverage() {
    let text = "Passano gli anni e il caso prende una direzione precisa. Alberto Stasi viene \
                rinviato a giudizio e il suo processo si sviluppa in un percorso che durerà \
                diversi anni. Nel duemilanove il giudice di primo grado pronuncia una prima \
                assoluzione, che nel duemilaundici viene confermata nel giudizio d'appello. \
                Nel duemilatredici la Cassazione interviene annullando con rinvio la sentenza, \
                e il caso torna in aula con un nuovo giudizio d'appello che si conclude nel \
                duemilaquattordici con una condanna a sedici anni di reclusione. \
                Nel duemilaquindici questa condanna diventa definitiva.";
    let value = manifest_value(vec![scene("sequenza", text)]);
    let scene_manifest = &value["scenes"][0];
    let sequences: Vec<&Value> = scene_manifest["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| group["group_type"] == "step_sequence")
        .collect();
    assert_eq!(
        sequences.len(),
        1,
        "one complete sequence expected: {value}"
    );
    let members = sequences[0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 5, "five steps expected: {members:?}");
    let dates: Vec<&str> = members
        .iter()
        .filter_map(|member| member["normalized_date"].as_str())
        .collect();
    let mut unique = dates.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(dates.len(), unique.len(), "no duplicated step: {dates:?}");

    let timelines: Vec<&Value> = scene_manifest["visual_proposals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|proposal| proposal["proposal_type"] == "timeline")
        .collect();
    assert_eq!(
        timelines.len(),
        1,
        "one timeline per sequence: {timelines:?}"
    );
    let end = timelines[0]["end_byte"].as_u64().unwrap();
    let last_step = members
        .iter()
        .find(|member| member["normalized_date"] == "2015")
        .expect("the 2015 step exists");
    assert!(
        last_step["end_byte"].as_u64().unwrap() <= end,
        "the timeline must reach the last step: {last_step}"
    );

    let coverage = &value["visual_coverage"];
    assert_eq!(
        coverage["covered_facts"], coverage["total_facts"],
        "every fact of a complete timeline is covered: {coverage}"
    );
}

#[test]
fn four_explicit_ordinal_steps_form_one_ordered_group_in_every_language() {
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "de",
            "Der Plan legte vier Schritte fest: erstens die Dichtungen tauschen, zweitens die Pumpen erneut prüfen, drittens die Sensoren nach jedem Regen kontrollieren, viertens die Ergebnisse am Saisonende veröffentlichen.",
            vec!["die Dichtungen tauschen", "die Pumpen erneut prüfen", "die Sensoren nach jedem Regen kontrollieren", "die Ergebnisse am Saisonende veröffentlichen"],
        ),
        (
            "en",
            "The plan fixed four steps: first replace the seals, second retest the pumps, third check the sensors after every rainfall, fourth publish the results at the end of the season.",
            vec!["replace the seals", "retest the pumps", "check the sensors after every rainfall", "publish the results at the end of the season"],
        ),
        (
            "es",
            "El acta fijó cuatro pasos: primero sustituir las juntas, segundo repetir las pruebas con la bomba, tercero revisar los sensores después de cada lluvia, cuarto publicar los resultados al final de la temporada.",
            vec!["sustituir las juntas", "repetir las pruebas con la bomba", "revisar los sensores después de cada lluvia", "publicar los resultados al final de la temporada"],
        ),
        (
            "fr",
            "Le procès-verbal fixe quatre étapes : d'abord remplacer les joints, ensuite refaire les essais avec la pompe, puis contrôler les capteurs après chaque pluie, enfin publier les résultats en fin de saison.",
            vec!["remplacer les joints", "refaire les essais avec la pompe", "contrôler les capteurs après chaque pluie", "publier les résultats en fin de saison"],
        ),
        (
            "id",
            "Notulen menetapkan empat langkah: pertama mengganti paking, kedua mengulang uji pompa, ketiga memeriksa sensor setelah setiap hujan, keempat menerbitkan hasil pada akhir musim.",
            vec!["mengganti paking", "mengulang uji pompa", "memeriksa sensor setelah setiap hujan", "menerbitkan hasil pada akhir musim"],
        ),
        (
            "it",
            "Il verbale comunale fissò quattro passaggi: primo, sostituire le guarnizioni; secondo, ripetere le prove con la pompa; terzo, controllare i sensori dopo ogni pioggia; quarto, pubblicare i risultati a fine stagione.",
            vec!["sostituire le guarnizioni", "ripetere le prove con la pompa", "controllare i sensori dopo ogni pioggia", "pubblicare i risultati a fine stagione"],
        ),
        (
            "nl",
            "Het verslag legde vier stappen vast: eerst de pakkingen vervangen, daarna de pompen opnieuw testen, vervolgens de sensoren na elke regenbui controleren, ten slotte de resultaten aan het einde van het seizoen publiceren.",
            vec!["de pakkingen vervangen", "de pompen opnieuw testen", "de sensoren na elke regenbui controleren", "de resultaten aan het einde van het seizoen publiceren"],
        ),
        (
            "pl",
            "Protokół ustalił cztery kroki: najpierw wymienić uszczelki, następnie powtórzyć próby z pompą, potem sprawdzać czujniki po każdym deszczu, na końcu opublikować wyniki pod koniec sezonu.",
            vec!["wymienić uszczelki", "powtórzyć próby z pompą", "sprawdzać czujniki po każdym deszczu", "opublikować wyniki pod koniec sezonu"],
        ),
        (
            "pt",
            "A ata fixou quatro passos: primeiro substituir as juntas, segundo repetir os ensaios com a bomba, terceiro verificar os sensores após cada chuva, quarto publicar os resultados no fim da época.",
            vec!["substituir as juntas", "repetir os ensaios com a bomba", "verificar os sensores após cada chuva", "publicar os resultados no fim da época"],
        ),
        (
            "ru",
            "Протокол закрепил четыре шага: сначала заменить прокладки, затем повторить испытания с насосом, после этого проверять датчики после каждого дождя, наконец опубликовать результаты в конце сезона.",
            vec!["заменить прокладки", "повторить испытания с насосом", "проверять датчики после каждого дождя", "опубликовать результаты в конце сезона"],
        ),
    ];
    for (language, text, expected) in cases {
        let value = manifest_value_for(language, vec![scene("ordinal-steps", text)]);
        let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
        let group = groups
            .iter()
            .find(|group| group["group_type"] == "step_sequence")
            .unwrap_or_else(|| panic!("{language}: ordered group expected: {value}"));
        let members = group["members"].as_array().unwrap();
        assert_eq!(members.len(), 4, "{language}: four members expected");
        assert_eq!(group["target_count"], 4);
        assert_eq!(group["complete"], true);
        for (index, (member, expected)) in members.iter().zip(expected).enumerate() {
            assert_eq!(member["rank"], index + 1);
            let member_text = member["text"].as_str().unwrap();
            assert!(
                member_text.contains(expected),
                "{language}: member {index} must contain {expected:?}: {member_text:?}"
            );
            assert_eq!(member["evidence"], member_text);
            let start = member["start_byte"].as_u64().unwrap() as usize;
            let end = member["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(
                text.get(start..end),
                Some(member_text),
                "{language}: member {index} must be a verbatim source slice"
            );
        }
    }
}

#[test]
fn sequence_adverbs_and_postfix_first_form_ordered_groups() {
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "de",
            "Die Reihenfolge blieb festgelegt: zuerst das Dach abdichten, dann die Fenster erneuern, danach die Heizung entlüften, schließlich die Regale zurückstellen.",
            vec!["das Dach abdichten", "die Fenster erneuern", "die Heizung entlüften", "die Regale zurückstellen"],
        ),
        (
            "en",
            "The order of works stayed fixed: seal the roof first, then replace the windows, afterwards bleed the radiators, finally return the shelves.",
            vec!["seal the roof", "replace the windows", "bleed the radiators", "return the shelves"],
        ),
        (
            "es",
            "El orden de los trabajos quedó fijado: primero sellar la cubierta, después cambiar las ventanas, luego purgar los radiadores, finalmente devolver las estanterías.",
            vec!["sellar la cubierta", "cambiar las ventanas", "purgar los radiadores", "devolver las estanterías"],
        ),
        (
            "id",
            "Urutan pekerjaan tetap: pertama menutup atap, kedua mengganti jendela, ketiga membuang udara radiator, terakhir mengembalikan rak.",
            vec!["menutup atap", "mengganti jendela", "membuang udara radiator", "mengembalikan rak"],
        ),
        (
            "pl",
            "Kolejność prac pozostała stała: najpierw uszczelnić dach, potem wymienić okna, następnie odpowietrzyć grzejniki, wreszcie odstawić regały.",
            vec!["uszczelnić dach", "wymienić okna", "odpowietrzyć grzejniki", "odstawić regały"],
        ),
        (
            "pt",
            "A ordem dos trabalhos ficou fixa: primeiro vedar o telhado, depois substituir as janelas, a seguir purgar os radiadores, por fim repor as estantes.",
            vec!["vedar o telhado", "substituir as janelas", "purgar os radiadores", "repor as estantes"],
        ),
    ];
    for (language, text, expected) in cases {
        let value = manifest_value_for(language, vec![scene("sequence-variants", text)]);
        let group = value["scenes"][0]["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["group_type"] == "step_sequence")
            .unwrap_or_else(|| panic!("{language}: sequence expected: {value}"));
        let members = group["members"].as_array().unwrap();
        assert_eq!(members.len(), 4, "{language}: four members expected");
        for (member, expected) in members.iter().zip(expected) {
            assert!(
                member["text"].as_str().unwrap().contains(expected),
                "{language}: expected {expected:?}: {member:?}"
            );
        }
    }
}

#[test]
fn fewer_than_four_or_unordered_steps_do_not_form_a_group() {
    let cases = [
        (
            "pl",
            "Protokół ustalił trzy kroki: najpierw wymienić uszczelki, następnie powtórzyć próby, potem sprawdzić czujniki.",
        ),
        (
            "en",
            "The plan fixed steps: first replace the seals and second retest the pumps.",
        ),
        (
            "it",
            "Il verbale ha fissato quattro passaggi: primo sostituire le guarnizioni; terzo controllare i sensori; quarto pubblicare i risultati.",
        ),
        (
            "ru",
            "Протокол закрепил шаги: сначала заменить прокладки, затем повторить испытания.",
        ),
        (
            "pl",
            "Lista: 3. wymienić uszczelki 1. powtórzyć próby 2. sprawdzić czujniki.",
        ),
    ];
    for (language, text) in cases {
        let value = manifest_value_for(language, vec![scene("not-a-sequence", text)]);
        assert!(
            value["scenes"][0]["semantic_groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|group| group["group_type"] != "step_sequence"),
            "{language}: no sequence expected for {text:?}: {value}"
        );
    }
}

#[test]
fn tied_ranking_is_not_a_step_sequence() {
    let text = "1. Anna 1. Marek 2. Zofia";
    let value = manifest_value_for("pl", vec![scene("tied-ranking", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    assert!(
        groups.iter().all(|group| group["group_type"] != "step_sequence"
            && group["group_type"] != "ranking"),
        "ties must not become a ranking or a sequence: {value}"
    );
}
