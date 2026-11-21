use visualner::{extract, ExtractOptions};

#[test]
#[ignore = "requires the local GLiNER2.5 model; run with BRAINRUST_NER_MODEL=gliner25 --ignored"]
fn gliner25_types_people_places_organizations_and_works_from_context() {
    assert!(
        std::env::var("BRAINRUST_NER_MODEL")
            .is_ok_and(|value| value.eq_ignore_ascii_case("gliner25")),
        "set BRAINRUST_NER_MODEL=gliner25 to run this model-backed test"
    );
    let model_dir = std::env::var_os("GLINER25_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("VISUALNER_MODEL_DIR")
                .map(std::path::PathBuf::from)
                .map(|path| path.join("gliner2.5-multi-v1-onnx"))
        })
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".local/share/visualner/models/gliner2.5-multi-v1-onnx")
        });
    assert!(
        ["fp16_25", "fp32_25"].iter().any(|variant| {
            model_dir
                .join(variant)
                .join("boundary_manifest.json")
                .is_file()
        }),
        "GLiNER2.5 model is missing under {}",
        model_dir.display()
    );

    let source = "Corrado Augias nacque a Roma. Nel 1987 il programma Telefono giallo fu trasmesso su Rai 3. Durante l’infanzia trascorse un periodo in Libia.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 30,
        },
    )
    .expect("extract with GLiNER2.5");

    for (surface, kind) in [
        ("Corrado Augias", "PERSON"),
        ("Roma", "LOCATION"),
        ("Telefono giallo", "WORK"),
        ("Rai 3", "ORG"),
        ("Libia", "LOCATION"),
    ] {
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == surface && entity.r#type == kind),
            "missing {kind} {surface:?}: {entities:?}"
        );
        assert!(!entities
            .iter()
            .any(|entity| { entity.text == surface && entity.r#type != kind }));
    }

    assert!(entities
        .iter()
        .all(|entity| { source.get(entity.start..entity.end) == Some(entity.text.as_str()) }));

    let disambiguation_source = "Il presidente della Repubblica Sergio Mattarella ricordò la figura storica di Gesù. Un altro programma fondamentale fu Babele. Collaborò con il settimanale *L'Espresso*.";
    let disambiguated = extract(
        disambiguation_source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 30,
        },
    )
    .expect("extract disambiguation sample with GLiNER2.5");
    assert!(
        !disambiguated.iter().any(|entity| {
            (entity.text == "Gesù" && entity.r#type == "LOCATION")
                || (entity.text == "Babele" && entity.r#type == "LOCATION")
                || entity.text == "Repubblica Sergio Mattarella"
                || (entity.text == "L'Espresso" && entity.r#type == "WORK")
        }),
        "legacy typing must not override GLiNER spans: {disambiguated:?}"
    );
}
