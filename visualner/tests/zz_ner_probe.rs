use gliner25_rs::{BoundaryConfig, BoundaryEngine, BoundaryParams, ExecutionMode, SchemaTask};

fn labels() -> Vec<String> {
    [
        "person",
        "organization",
        "location",
        "book",
        "television program",
        "play",
        "award",
        "historical event",
        "facility",
        "product",
        "brand",
    ]
    .iter()
    .map(|label| (*label).to_string())
    .collect()
}

fn hits(engine: &mut BoundaryEngine, source: &str, want: &str, offset: usize) -> Vec<String> {
    let tasks = [SchemaTask::Entities(labels())];
    let params = BoundaryParams {
        threshold: 0.05,
        ..BoundaryParams::default()
    };
    let out = engine
        .extract_long_with(source, &tasks, &params, Default::default())
        .expect("extract");
    let mut rows: Vec<(f32, String)> = out
        .mentions
        .iter()
        .filter(|m| m.score >= 0.05)
        .filter(|m| {
            let text = &source[m.char_start..m.char_end];
            want.split_whitespace().any(|part| text.contains(part))
                || text.contains(want)
                || matches!(m.field.as_str(), "brand" | "organization" | "company")
                    && want.contains(text)
        })
        .map(|m| {
            (
                m.score,
                format!(
                    "{:?}={:.2}@{}",
                    &source[m.char_start..m.char_end],
                    m.score,
                    m.field
                ),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let _ = offset;
    rows.into_iter().map(|(_, row)| row).collect()
}

#[test]
#[ignore]
fn probe_long_context() {
    let model_dir = std::env::var_os("GLINER25_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(
                "/home/pierone/.local/share/visualner/models/gliner2.5-multi-v1-onnx",
            )
        });
    for name in ["libonnxruntime.so", "libonnxruntime.so.1.23.2"] {
        let candidate = model_dir.join(name);
        if candidate.is_file() {
            std::env::set_var("ORT_DYLIB_PATH", candidate);
            break;
        }
    }
    gliner25_rs::init("probe");
    let mut engine = BoundaryEngine::new(
        BoundaryConfig::new(&model_dir).with_execution(ExecutionMode::Standard),
    )
    .expect("load");

    for (file, wanted) in [
        ("de_stadion_sued.txt", ["Büro Steinbach", "Büro Auerbach"]),
        ("es_estadio_norte.txt", ["Agua Norte", "Energía Puerto"]),
        ("pt_estadio_norte.txt", ["Água Norte", "Energia Porto"]),
    ] {
        let path = format!("../semantic-engine/fixtures/gold_multilingual/long_scripts/{file}");
        let text = std::fs::read_to_string(&path).expect("fixture");
        let full = text.lines().next().expect("line").to_string();
        println!("### {file} FULL ({} chars)", full.chars().count());
        for want in wanted {
            println!("  {want:?} -> {:?}", hits(&mut engine, &full, want, 0));
        }
        println!("### {file} PER SENTENCE");
        for sentence in full.split_inclusive(['.', '!', '?']) {
            if !wanted.iter().any(|want| sentence.contains(want)) {
                continue;
            }
            for want in wanted {
                if sentence.contains(want) {
                    println!(
                        "  {want:?} -> {:?}",
                        hits(&mut engine, sentence.trim(), want, 0)
                    );
                }
            }
        }
    }
}
