use script_qa::dates::{explicit_calendar_dates, CalendarDate};
use script_qa::gate::{
    compute_clip_evidence_coverage, evaluate, policy_thresholds, source_date_preservation_reasons,
};
use script_qa::model::{
    ArtifactsInput, ClipBindingInput, ClipEvidenceInput, EntitiesInput, EntityInput, Evaluation,
    GateRequest, ItemInput, OutputInput, PlanInput, ResearchCandidateInput, ResearchEvidenceInput,
    ResultInput, SceneBindingsInput, SceneInput, SegmentInput, SpecSceneInput,
};
use script_qa::tokenize::{compute_source_text_coverage, detect_language};

fn request(plan: PlanInput, result: ResultInput) -> GateRequest {
    GateRequest {
        item: ItemInput {
            id: "item-1".to_string(),
        },
        plan,
        result: Some(result),
    }
}

fn plan(language: &str, source_text: &str) -> PlanInput {
    PlanInput {
        language: language.to_string(),
        source_text: source_text.to_string(),
        ..PlanInput::default()
    }
}

fn result(text: &str, word_count: i32) -> ResultInput {
    ResultInput {
        output: OutputInput {
            text: text.to_string(),
            word_count,
            specscene: SpecSceneInput::default(),
        },
        artifacts: ArtifactsInput::default(),
    }
}

fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        id: id.to_string(),
        text: text.to_string(),
        ..SceneInput::default()
    }
}

fn segment_scene(id: &str, segment_id: &str, text: &str) -> SceneInput {
    SceneInput {
        id: id.to_string(),
        segment_id: segment_id.to_string(),
        text: text.to_string(),
        ..SceneInput::default()
    }
}

fn clip_scene(id: &str, clip_id: &str) -> SceneInput {
    SceneInput {
        id: id.to_string(),
        bindings: SceneBindingsInput {
            clip: Some(ClipBindingInput {
                clip_id: clip_id.to_string(),
            }),
        },
        ..SceneInput::default()
    }
}

fn evaluate_ok(request: &GateRequest) -> Evaluation {
    evaluate(request).expect("the fixture always carries a result")
}

fn word_block(topic: &str) -> String {
    format!("{topic} tecnica disciplina storia ").repeat(25)
}

#[test]
fn passes_clean_generation() {
    let request = request(
        PlanInput {
            target_words: 10,
            ..plan(
                "en",
                "The quick brown fox jumps over the lazy dog in Italy.",
            )
        },
        result("The quick brown fox jumps over the lazy dog in Italy.", 10),
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
    assert_eq!(evaluation.quality.language_requested, "en");
    assert_eq!(evaluation.quality.language_detected, "en");
    assert!(evaluation.quality.source_text_coverage >= 0.70);
    assert_eq!(evaluation.quality.clip_evidence_coverage, 1.0);
    assert_eq!(evaluation.quality.unsupported_claims, 0);
    assert_eq!(evaluation.quality.target_words, 10);
    assert_eq!(evaluation.quality.actual_words, 10);
}

#[test]
fn rejects_cross_boxer_segment_text() {
    let segments = vec![
        SegmentInput {
            id: "boxer-mike-tyson".to_string(),
            topic: "Mike Tyson".to_string(),
            ..SegmentInput::default()
        },
        SegmentInput {
            id: "boxer-muhammad-ali".to_string(),
            topic: "Muhammad Ali".to_string(),
            ..SegmentInput::default()
        },
        SegmentInput {
            id: "boxer-evander-holyfield".to_string(),
            topic: "Evander Holyfield".to_string(),
            ..SegmentInput::default()
        },
        SegmentInput {
            id: "boxer-floyd-mayweather".to_string(),
            topic: "Floyd Mayweather".to_string(),
            ..SegmentInput::default()
        },
        SegmentInput {
            id: "boxer-sugar-ray-robinson".to_string(),
            topic: "Sugar Ray Robinson".to_string(),
            ..SegmentInput::default()
        },
    ];
    let scenes = segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let text = if segment.id == "boxer-floyd-mayweather" {
                word_block("Sugar Ray Robinson")
            } else {
                word_block(&segment.topic)
            };
            segment_scene(&format!("scene-{index}"), &segment.id, &text)
        })
        .collect();

    let request = request(
        PlanInput {
            segments,
            ..PlanInput::default()
        },
        ResultInput {
            output: OutputInput {
                text: "boxers documentary".to_string(),
                word_count: 625,
                specscene: SpecSceneInput { scenes },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert!(
        evaluation
            .reasons
            .iter()
            .any(|reason| reason.contains("boxer-floyd-mayweather")),
        "{:?}",
        evaluation.reasons
    );
}

#[test]
fn fails_language_mismatch() {
    let request = request(
        plan("en", "The cat jumps on the table and looks at the sky."),
        result("Il gatto salta sul tavolo e guarda il cielo.", 9),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert_eq!(evaluation.quality.language_detected, "it");
    assert_eq!(evaluation.quality.language_requested, "en");
}

#[test]
fn language_match_rejects_mixed_language_scene() {
    let english_scene = "The late afternoon sun casts long shadows as Isabelle arrives at the condominium entrance. The visual evidence captured by the cameras documents this arrival and fixes a precise point in time. However, viewers must understand the limitations inherent in any recorded sequence, because it cannot answer what happened after she entered.";
    let portuguese_script = "Isabelle era uma jovem estudante de Direito. O caso segue em investigação, e os fatos confirmados precisam ser separados das hipóteses. As autoridades continuam analisando depoimentos, perícias e imagens para esclarecer o que ocorreu.";
    let request = request(
        plan("pt", ""),
        ResultInput {
            output: OutputInput {
                text: format!("{portuguese_script} {english_scene}"),
                word_count: 0,
                specscene: SpecSceneInput {
                    scenes: vec![
                        scene("scene-1", portuguese_script),
                        scene("scene-2", english_scene),
                    ],
                },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(
        evaluation
            .reasons
            .iter()
            .any(|reason| reason.contains("scene scene-2 detected language en")),
        "{:?}",
        evaluation.reasons
    );
}

#[test]
fn fails_source_text_coverage() {
    let request = request(
        plan("en", "The quick brown fox jumps over the lazy dog."),
        result(
            "A completely different story about space travel and aliens.",
            9,
        ),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert!(evaluation.quality.source_text_coverage < 0.70);
}

#[test]
fn fails_target_words_tolerance() {
    let request = request(
        PlanInput {
            target_words: 100,
            ..plan("en", "The quick brown fox jumps over the lazy dog.")
        },
        result("The quick brown fox.", 4),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert_eq!(evaluation.quality.actual_words, 4);
    assert_eq!(evaluation.quality.target_words, 100);
}

#[test]
fn allows_short_clip_introduction() {
    let text = "Margot says she would spend the day with J.K. Rowling asking a million questions.";
    let request = request(
        PlanInput {
            target_words: 100,
            segments: vec![SegmentInput {
                id: "scene-0".to_string(),
                topic: "J.K. Rowling".to_string(),
                ..SegmentInput::default()
            }],
            clip_evidence: Some(ClipEvidenceInput::default()),
            grounding_policy: "clips_primary".to_string(),
            ..plan(
                "en",
                "Margot says she would spend the day with J.K. Rowling asking a million and 73 questions.",
            )
        },
        ResultInput {
            output: OutputInput {
                text: text.to_string(),
                word_count: 14,
                specscene: SpecSceneInput {
                    scenes: vec![scene("scene-0", text)],
                },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
}

#[test]
fn fails_empty_text() {
    let request = request(plan("en", ""), result("   ", 0));

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
}

#[test]
fn fails_generic_text() {
    let request = request(
        plan("en", ""),
        result("This is a sample text placeholder for the video.", 9),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
}

#[test]
fn source_absent_is_not_evaluated() {
    let request = request(
        plan("en", ""),
        result("The generated documentary narrative is complete.", 6),
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
    assert_eq!(
        evaluation.quality.source_text_coverage_status,
        "NOT_EVALUATED"
    );
    assert_ne!(evaluation.quality.source_text_coverage, 1.0);
}

#[test]
fn fails_clip_evidence_coverage() {
    let request = request(
        PlanInput {
            grounding_policy: "clips_primary".to_string(),
            clip_evidence: Some(ClipEvidenceInput {
                accepted_clip_ids: vec!["clip-1".to_string(), "clip-2".to_string()],
                ..ClipEvidenceInput::default()
            }),
            ..plan("en", "Scene one uses clip one.")
        },
        ResultInput {
            output: OutputInput {
                text: "Scene one uses clip one.".to_string(),
                word_count: 5,
                specscene: SpecSceneInput {
                    scenes: vec![clip_scene("scene-1", "clip-1")],
                },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert_eq!(evaluation.quality.clip_evidence_coverage, 0.5);
}

#[test]
fn reports_unsupported_claims_for_grounded_source() {
    let text = "Marco Polo visited Venice and discovered new lands.";
    let request = request(
        PlanInput {
            source_kind: "research".to_string(),
            ..plan("en", text)
        },
        ResultInput {
            output: OutputInput {
                text: text.to_string(),
                word_count: 9,
                specscene: SpecSceneInput::default(),
            },
            artifacts: ArtifactsInput {
                entities: Some(EntitiesInput {
                    persons: vec![EntityInput {
                        value: "Marco Polo".to_string(),
                    }],
                    places: vec![EntityInput {
                        value: "Venice".to_string(),
                    }],
                    concepts: vec![
                        EntityInput {
                            value: "new lands".to_string(),
                        },
                        EntityInput {
                            value: "unknown concept".to_string(),
                        },
                    ],
                }),
            },
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
    assert_eq!(evaluation.quality.unsupported_claims, 1);
}

#[test]
fn text_unsupported_claims_are_diagnostic() {
    let text = "Marco Polo visited Venice and discovered new lands.";
    let request = request(
        PlanInput {
            source_kind: "text".to_string(),
            ..plan("en", text)
        },
        ResultInput {
            output: OutputInput {
                text: text.to_string(),
                word_count: 9,
                specscene: SpecSceneInput::default(),
            },
            artifacts: ArtifactsInput {
                entities: Some(EntitiesInput {
                    persons: vec![EntityInput {
                        value: "Marco Polo".to_string(),
                    }],
                    places: vec![EntityInput {
                        value: "Venice".to_string(),
                    }],
                    concepts: vec![
                        EntityInput {
                            value: "new lands".to_string(),
                        },
                        EntityInput {
                            value: "unknown concept".to_string(),
                        },
                    ],
                }),
            },
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
    assert_eq!(evaluation.quality.unsupported_claims, 1);
}

#[test]
fn computes_source_text_coverage() {
    let generated = "the quick brown fox jumps";
    let source = "the quick brown fox jumps over the lazy dog";
    assert_eq!(compute_source_text_coverage(generated, source), 1.0);

    let generated = "space travel aliens unknown planet";
    let source = "the quick brown fox jumps over the lazy dog";
    assert!(compute_source_text_coverage(generated, source) < 0.70);
}

#[test]
fn detects_language() {
    let cases = [
        ("The quick brown fox jumps over the lazy dog.", "en"),
        ("Il gatto salta sul tavolo.", "it"),
        ("El gato salta sobre la mesa.", "es"),
        ("Le chat saute sur la table.", "fr"),
        ("Der Hund läuft im Park.", "de"),
        ("xyz qwerty 12345", ""),
    ];
    for (text, wanted) in cases {
        assert_eq!(detect_language(text), wanted, "{text}");
    }
}

#[test]
fn policy_thresholds_are_stable() {
    let cases = [
        ("clips_primary", 0.40, 1.00),
        ("source_primary", 0.85, 0.00),
        ("balanced", 0.60, 0.50),
        ("", 0.70, 0.00),
    ];
    for (policy, wanted_source, wanted_clip) in cases {
        let (source, clip) = policy_thresholds(policy);
        assert_eq!(source, wanted_source, "{policy}");
        assert_eq!(clip, wanted_clip, "{policy}");
    }
}

#[test]
fn source_primary_requires_high_text_coverage() {
    let text = "The quick brown fox jumps over the lazy dog in Italy.";
    let request = request(
        PlanInput {
            target_words: 10,
            grounding_policy: "source_primary".to_string(),
            ..plan("en", text)
        },
        result(text, 10),
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
}

#[test]
fn source_primary_fails_low_text_coverage() {
    let request = request(
        PlanInput {
            grounding_policy: "source_primary".to_string(),
            ..plan("en", "The quick brown fox jumps over the lazy dog.")
        },
        result(
            "A completely different story about space travel and aliens.",
            9,
        ),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
}

#[test]
fn clips_primary_passes_low_text_coverage_with_full_clip_binding() {
    let text = "The quick brown fox jumps over the lazy dog in Italy.";
    let request = request(
        PlanInput {
            grounding_policy: "clips_primary".to_string(),
            clip_evidence: Some(ClipEvidenceInput {
                accepted_clip_ids: vec!["clip-1".to_string()],
                ..ClipEvidenceInput::default()
            }),
            ..plan("en", text)
        },
        ResultInput {
            output: OutputInput {
                text: text.to_string(),
                word_count: 10,
                specscene: SpecSceneInput {
                    scenes: vec![clip_scene("scene-1", "clip-1")],
                },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
}

#[test]
fn balanced_fails_when_clip_coverage_low() {
    let text = "The quick brown fox jumps over the lazy dog.";
    let request = request(
        PlanInput {
            grounding_policy: "balanced".to_string(),
            clip_evidence: Some(ClipEvidenceInput {
                accepted_clip_ids: vec![
                    "clip-1".to_string(),
                    "clip-2".to_string(),
                    "clip-3".to_string(),
                ],
                ..ClipEvidenceInput::default()
            }),
            ..plan("en", text)
        },
        ResultInput {
            output: OutputInput {
                text: text.to_string(),
                word_count: 9,
                specscene: SpecSceneInput {
                    scenes: vec![clip_scene("scene-1", "clip-1")],
                },
            },
            artifacts: ArtifactsInput::default(),
        },
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
}

#[test]
fn computes_clip_evidence_coverage() {
    let plan = PlanInput {
        clip_evidence: Some(ClipEvidenceInput {
            accepted_clip_ids: vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
            ..ClipEvidenceInput::default()
        }),
        ..PlanInput::default()
    };
    let result = ResultInput {
        output: OutputInput {
            specscene: SpecSceneInput {
                scenes: vec![clip_scene("scene-1", "c1"), clip_scene("scene-2", "c2")],
            },
            ..OutputInput::default()
        },
        ..ResultInput::default()
    };
    assert_eq!(compute_clip_evidence_coverage(&plan, &result), 2.0 / 3.0);
}

#[test]
fn single_segment_moderate_coverage_passes() {
    const GENERATED: &str = "Sugar Ray Robinson boxer American career investments legacy heritage";
    const SOURCE: &str = "Sugar Ray Robinson boxer American career investments legacy heritage";
    let request = request(
        PlanInput {
            target_words: 10,
            ..plan("", SOURCE)
        },
        result(GENERATED, 9),
    );

    let evaluation = evaluate_ok(&request);
    assert!(evaluation.quality.passed, "{:?}", evaluation.reasons);
}

#[test]
fn single_segment_sub_threshold_fails() {
    let request = request(
        PlanInput {
            target_words: 10,
            ..plan("", "Sugar Ray Robinson boxer American career")
        },
        result("space travel aliens unknown planet", 9),
    );

    let evaluation = evaluate_ok(&request);
    assert!(!evaluation.quality.passed);
}

#[test]
fn source_date_preservation_rejects_changed_year() {
    let plan = PlanInput {
        segments: vec![SegmentInput {
            id: "delitto-e-luogo".to_string(),
            source_text: "Presenta Chiara Poggi uccisa a Garlasco il 13 agosto 2007.".to_string(),
            ..SegmentInput::default()
        }],
        ..PlanInput::default()
    };
    let result = ResultInput {
        output: OutputInput {
            text: "Il tredici agosto del duemiladue è una data ricordata a Garlasco.".to_string(),
            specscene: SpecSceneInput {
                scenes: vec![segment_scene(
                    "scene-1",
                    "delitto-e-luogo",
                    "Il tredici agosto del duemiladue è una data ricordata a Garlasco.",
                )],
            },
            ..OutputInput::default()
        },
        ..ResultInput::default()
    };

    let reasons = source_date_preservation_reasons(&result, &plan);
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].contains("13-08-2007"), "{reasons:?}");
}

#[test]
fn source_date_preservation_accepts_equivalent_formats() {
    for narration in [
        "Il 13 agosto 2007 è un riferimento temporale del caso.",
        "La data, 13 agosto del 2007, è riportata nella fonte.",
        "Il 13/08/2007 è un riferimento temporale del caso.",
    ] {
        let plan = PlanInput {
            segments: vec![SegmentInput {
                id: "delitto-e-luogo".to_string(),
                source_text: "Il fatto risale al 13 agosto 2007.".to_string(),
                ..SegmentInput::default()
            }],
            ..PlanInput::default()
        };
        let result = ResultInput {
            output: OutputInput {
                text: narration.to_string(),
                specscene: SpecSceneInput {
                    scenes: vec![segment_scene("scene-1", "delitto-e-luogo", narration)],
                },
                ..OutputInput::default()
            },
            ..ResultInput::default()
        };
        assert!(
            source_date_preservation_reasons(&result, &plan).is_empty(),
            "{narration}"
        );
    }
}

#[test]
fn explicit_calendar_dates_normalize_numeric_separators() {
    let wanted = CalendarDate {
        day: 13,
        month: 8,
        year: 2007,
    };
    for text in ["13/08/2007", "13-08-2007", "13.08.2007", "13 agosto 2007"] {
        assert_eq!(explicit_calendar_dates(text), vec![wanted], "{text}");
    }
}

#[test]
fn research_candidate_coverage_fails_when_candidate_is_omitted() {
    let request = request(
        PlanInput {
            research_evidence: Some(ResearchEvidenceInput {
                candidates: vec![
                    ResearchCandidateInput {
                        label: "Cristiano Ronaldo".to_string(),
                    },
                    ResearchCandidateInput {
                        label: "Canelo Álvarez".to_string(),
                    },
                ],
            }),
            ..PlanInput::default()
        },
        result("Cristiano Ronaldo e Lionel Messi hanno grandi guadagni.", 0),
    );

    let evaluation = evaluate_ok(&request);
    assert!(
        evaluation
            .reasons
            .iter()
            .any(|reason| reason.contains("ranked candidate(s)")),
        "{:?}",
        evaluation.reasons
    );
}

#[test]
fn research_candidate_coverage_normalizes_accents() {
    let request = request(
        PlanInput {
            research_evidence: Some(ResearchEvidenceInput {
                candidates: vec![ResearchCandidateInput {
                    label: "Canelo Álvarez".to_string(),
                }],
            }),
            ..PlanInput::default()
        },
        result("Canelo Alvarez ha costruito una grande carriera.", 0),
    );

    let evaluation = evaluate_ok(&request);
    assert!(
        !evaluation
            .reasons
            .iter()
            .any(|reason| reason.contains("ranked candidate(s)")),
        "{:?}",
        evaluation.reasons
    );
}
