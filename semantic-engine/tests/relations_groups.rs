//! Comparison, cause-effect and co-mention semantic groups.

mod common;

use common::*;
#[test]
fn explicit_collaboration_marker_produces_a_grounded_asserted_relation() {
    let text = "Alberto Stasi collabora con Andrea Sempio.";
    let value = manifest_value(vec![scene("collaborazione", text)]);
    let relations = value["scenes"][0]["relations"]["asserted_relations"]
        .as_array()
        .unwrap();
    assert_eq!(relations.len(), 1, "explicit predicate only: {value}");
    assert_eq!(relations[0]["predicate"], "collaboration");
    assert_eq!(relations[0]["status"], "Asserted");
    assert_eq!(relations[0]["evidence"], text);
}

#[test]
fn explicit_comparison_cue_creates_a_grounded_comparison_group() {
    let text = "Alberto Stasi versus Andrea Sempio.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "comparison")
        .expect("explicit comparison marker and two entities should form a group");
    let members = group["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        2,
        "unexpected comparison members: {members:?}"
    );
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn co_mention_without_explicit_comparison_cue_does_not_form_a_comparison_group() {
    let text = "Alberto Stasi and Andrea Sempio are mentioned in the same report.";
    let value = manifest_value(vec![scene("co-mention", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "comparison"));
}

#[test]
fn repeated_explicit_comparison_links_grounded_members_across_scenes() {
    let inputs = vec![
        scene(
            "comparison-first",
            "Lo Stadio Aurora è più capiente rispetto allo Stadio Olimpico.",
        ),
        scene(
            "comparison-second",
            "Lo Stadio Aurora è più antico rispetto allo Stadio Olimpico.",
        ),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let group = groups
        .iter()
        .find(|group| {
            group["group_type"] == "comparison"
                && group["ranking_basis"] == "explicit_pair_comparison_across_scenes"
        })
        .unwrap_or_else(|| {
            panic!("same explicitly compared identity pair should be joined across scenes: {value}")
        });
    assert_eq!(group["members"].as_array().unwrap().len(), 4);
    for member in group["members"].as_array().unwrap() {
        let scene_id = member["scene_id"].as_str().unwrap();
        let source = &inputs
            .iter()
            .find(|input| input.scene_id == scene_id)
            .expect("member scene must belong to this request")
            .text;
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(source.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn repeated_co_mentions_without_explicit_comparison_do_not_form_cross_scene_comparison() {
    let inputs = vec![
        scene(
            "co-mention-first",
            "Stadio Aurora e Stadio Olimpico sono impianti distinti.",
        ),
        scene(
            "co-mention-second",
            "Stadio Aurora e Stadio Olimpico sono stati citati.",
        ),
    ];
    let value = manifest_value(inputs);
    assert!(value["scenes"].as_array().unwrap().iter().all(|scene| {
        scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| {
                group["group_type"] != "comparison"
                    || group["ranking_basis"] != "explicit_pair_comparison_across_scenes"
            })
    }));
}

#[test]
fn asserted_causal_relation_produces_a_cause_effect_group() {
    let text = "Lo Stadio Aurora causa l'espansione di Torino.";
    let value = manifest_value(vec![scene("causa-effetto", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "cause_effect")
        .expect("explicit positive causal relation should form a group");
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
}

#[test]
fn negated_or_uncertain_causal_relation_does_not_form_a_cause_effect_group() {
    for text in [
        "Lo Stadio Aurora non causa l'espansione di Torino.",
        "Forse lo Stadio Aurora causa l'espansione di Torino.",
    ] {
        let value = manifest_value(vec![scene("causa-non-certa", text)]);
        assert!(value["scenes"][0]["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "cause_effect"));
    }
}

#[test]
fn explicit_causal_chain_links_grounded_evidence_across_scenes() {
    let inputs = vec![
        scene(
            "cause-origin",
            "Lo Stadio Aurora causa la crescita di Torino.",
        ),
        scene("cause-next", "Torino causa la crescita di Milano."),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let group = groups
        .iter()
        .find(|group| {
            group["group_type"] == "cause_effect"
                && group["ranking_basis"] == "explicit_asserted_causal_chain_across_scenes"
        })
        .expect("explicit causal edges sharing an endpoint should form a cross-scene chain");
    assert_eq!(group["members"].as_array().unwrap().len(), 4);
    for member in group["members"].as_array().unwrap() {
        let scene_id = member["scene_id"].as_str().unwrap();
        let source = &inputs
            .iter()
            .find(|input| input.scene_id == scene_id)
            .expect("member scene must belong to this request")
            .text;
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(source.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn negated_second_causal_edge_does_not_form_cross_scene_chain() {
    let inputs = vec![
        scene(
            "cause-origin",
            "Lo Stadio Aurora causa la crescita di Torino.",
        ),
        scene("cause-next", "Torino non causa la crescita di Milano."),
    ];
    let value = manifest_value(inputs);
    assert!(value["scenes"].as_array().unwrap().iter().all(|scene| {
        scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| {
                group["group_type"] != "cause_effect"
                    || group["ranking_basis"] != "explicit_asserted_causal_chain_across_scenes"
            })
    }));
}

#[test]
fn a_noun_list_with_a_comparison_cue_is_not_a_comparison() {
    // The cue alone used to turn every entity of the sentence into a member,
    // so a paragraph listing eleven nouns became an eleven-way comparison.
    let text = "La vicenda è più complicata rispetto alla cronologia, e coinvolge \
                colpevolezza, diritto, procedura, testimonianze, atti, sentenze, \
                perizie, indagini e dibattimento.";
    let value = manifest_value(vec![scene("noun-list", text)]);
    let groups = &value["scenes"][0]["semantic_groups"];
    assert!(
        groups
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "comparison"),
        "generic nouns are not comparable subjects: {groups}"
    );
}

#[test]
fn a_comparison_has_exactly_the_two_named_subjects_around_the_cue() {
    let text = "Garlasco è più piccolo rispetto a Pavia, ma il caso ha avuto la stessa risonanza.";
    let value = manifest_value(vec![scene("two-places", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "comparison")
        .unwrap_or_else(|| panic!("two named places with a cue must compare: {value}"));
    let members = group["members"].as_array().unwrap();
    let texts: Vec<&str> = members
        .iter()
        .map(|member| member["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["Garlasco", "Pavia"], "members: {members:?}");
}
