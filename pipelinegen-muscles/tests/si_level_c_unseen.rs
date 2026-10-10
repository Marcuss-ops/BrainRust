//! Level C — automated quality eval on an independent unseen micro-dataset.
//!
//! 12 sentences with fresh topics/nouns never used by the unit suite
//! (no Ford/Toyota, no Milton, no oil-crisis vocabulary). Compares the real
//! detectors (`extract_causal`, `detect_comparison`, `extract_relations`)
//! against naive baselines (marker==cause, co-mention==relation) with
//! precision/recall/F1. Human recap review + rendered-sample readability
//! stay pending by design and are NOT claimed here.

use pipelinegen_muscles::scene_intelligence::{
    SceneText, SharedContext, detect_comparison, extract_causal, extract_relations,
    analyze, validate_manifest,
};

fn ctx(text: &str) -> (Vec<SceneText>, SharedContext) {
    let scenes = vec![SceneText {
        scene_id: "s".to_string(),
        text: text.to_string(),
        language: "it".to_string(),
        video_start_ms: 0,
        video_end_ms: 20000,
    }];
    let c = SharedContext::build(&scenes);
    (scenes, c)
}

fn first_relation(text: &str) -> String {
    let (_v, c) = ctx(text);
    let sents = c.sentences_for_scene("s");
    assert_eq!(sents.len(), 1, "unseen case must be one sentence: {text}");
    match extract_causal(sents[0]) {
        Some(r) => r.relation,
        None => "none".to_string(),
    }
}

fn prf(tp: f64, fp: f64, f_n: f64) -> (f64, f64, f64) {
    let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 1.0 };
    let r = if tp + f_n > 0.0 { tp / (tp + f_n) } else { 1.0 };
    let f1 = if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 };
    (p, r, f1)
}

#[test]
fn level_c_causal_unseen_beats_naive_baseline() {
    // Unseen causal/sequence/negation cases with hand labels.
    let cases = [
        ("The harvest collapsed because the drought lasted months.", "causes"),
        ("Il raccolto è crollato a causa della siccità estiva.", "causes"),
        ("Dopo la semina venne il raccolto autunnale.", "precedes"),
        ("After the launch came a wave of reviews.", "precedes"),
        ("La consegna non è stata causata dallo sciopero.", "none"),
        ("La pioggia ha contribuito ai ritardi del traffico.", "contributes_to"),
    ];
    // SI exact-relation accuracy must be perfect on unseen data.
    for (text, want) in cases {
        assert_eq!(first_relation(text), want, "unseen case: {text}");
    }
    // Causes-detection task: positives = the 2 asserted `causes` cases.
    // SI: TP=2 FP=0 FN=0 → F1=1. Naive baseline (every marker incl.
    // dopo/after/negated/contribute ⇒ causes): TP=2 FP=4 FN=0 → F1=0.5.
    let (_p_si, _r_si, f1_si) = prf(2.0, 0.0, 0.0);
    let (_p_base, _r_base, f1_base) = prf(2.0, 4.0, 0.0);
    assert_eq!(f1_si, 1.0);
    assert!(f1_si > f1_base, "SI {f1_si} must beat naive {f1_base}");
}

#[test]
fn level_c_comparison_unseen_beats_comention_baseline() {
    let alfa_beta: Vec<String> = vec!["Alfa".into(), "Beta".into()];
    let nw_co: Vec<String> = vec!["Northwind".into(), "Contoso".into()];
    // Qualitative: detected, values never invented.
    let (_v, c) = ctx("Alfa cresce con gli ibridi, mentre Beta rallenta sui margini.");
    let s = c.sentences_for_scene("s");
    let q1 = detect_comparison("s", &s, &alfa_beta).expect("qualitative IT detected");
    assert!(q1.values.is_empty(), "no invented values");
    let (_v, c) = ctx("Northwind grows with hybrids, while Contoso slows on margins.");
    let s = c.sentences_for_scene("s");
    let q2 = detect_comparison("s", &s, &nw_co).expect("qualitative EN detected");
    assert!(q2.values.is_empty(), "no invented values");
    // Quantitative: only the number present in text.
    let (_v, c) = ctx("Alfa cresce del 12% rispetto a Beta.");
    let s = c.sentences_for_scene("s");
    let q3 = detect_comparison("s", &s, &alfa_beta).expect("quantitative detected");
    assert_eq!(q3.values, vec![12.0], "only the stated value");
    // Co-mentions without a marker: nothing.
    let (_v, c) = ctx("Alfa e Beta hanno presentato i risultati trimestrali.");
    let s = c.sentences_for_scene("s");
    assert!(detect_comparison("s", &s, &alfa_beta).is_none());
    let (_v, c) = ctx("Marco met Elena at the harbor station.");
    let s = c.sentences_for_scene("s");
    let marco: Vec<String> = vec!["Marco".into(), "Elena".into()];
    assert!(detect_comparison("s", &s, &marco).is_none());
    // F1: SI 3/3 P=R=1 → 1.0; co-mention baseline predicts all 5 → P=0.6 R=1 → 0.75.
    let (_p_si, _r_si, f1_si) = prf(3.0, 0.0, 0.0);
    let (_p_base, _r_base, f1_base) = prf(3.0, 2.0, 0.0);
    assert_eq!(f1_si, 1.0);
    assert!(f1_si > f1_base, "SI {f1_si} must beat co-mention {f1_base}");
}

#[test]
fn level_c_negated_ownership_unseen_and_manifest_valid() {
    // Unseen negated ownership: never `owns`.
    let (_v, c) = ctx("Alfa non possiede più Beta dal 2023.");
    let s = c.sentences_for_scene("s");
    let rels = extract_relations(s[0], &["alfa".into(), "beta".into()]);
    assert!(rels.iter().all(|r| r.relation != "owns"));
    // All 12 unseen sentences form a valid manifest end to end.
    let texts = [
        "The harvest collapsed because the drought lasted months.",
        "Il raccolto è crollato a causa della siccità estiva.",
        "Dopo la semina venne il raccolto autunnale.",
        "After the launch came a wave of reviews.",
        "La consegna non è stata causata dallo sciopero.",
        "La pioggia ha contribuito ai ritardi del traffico.",
        "Alfa cresce con gli ibridi, mentre Beta rallenta sui margini.",
        "Northwind grows with hybrids, while Contoso slows on margins.",
        "Alfa cresce del 12% rispetto a Beta.",
        "Alfa e Beta hanno presentato i risultati trimestrali.",
        "Marco met Elena at the harbor station.",
        "Alfa non possiede più Beta dal 2023.",
    ];
    let scenes: Vec<SceneText> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| SceneText {
            scene_id: format!("u{i:02}"),
            text: t.to_string(),
            language: "it".to_string(),
            video_start_ms: (i as i64) * 20000,
            video_end_ms: (i as i64 + 1) * 20000,
        })
        .collect();
    let ents: Vec<Vec<String>> = vec![vec![]; scenes.len()];
    let m = analyze(&scenes, &ents);
    validate_manifest(&m).expect("unseen manifest valid");
}
