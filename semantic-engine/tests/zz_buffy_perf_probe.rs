use std::time::Instant;

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};

fn synthetic_scene() -> String {
    let firsts = [
        "Mario", "Luca", "Andrea", "Paolo", "Giulia", "Sara", "Chiara", "Marco", "Elena", "Davide",
    ];
    let lasts = [
        "Rossi",
        "Bianchi",
        "Ferrari",
        "Esposito",
        "Romano",
        "Colombo",
        "Ricci",
        "Marino",
        "Greco",
        "Bruno",
        "Gallo",
        "Conti",
        "De Luca",
        "Mancini",
        "Costa",
        "Giordano",
        "Rizzo",
        "Lombardi",
        "Moretti",
        "Barbieri",
        "Fontana",
        "Santoro",
        "Mariani",
        "Rinaldi",
        "Caruso",
        "Ferrara",
        "Galli",
        "Martini",
        "Leone",
        "Longo",
        "Gentile",
        "Martinelli",
        "Vitale",
        "Lombardo",
        "Serra",
        "Coppola",
        "De Santis",
        "D'Angelo",
        "Marchetti",
        "Parisi",
        "Villa",
        "Conte",
        "Ferraro",
        "Ferri",
        "Fabbri",
        "Bianco",
        "Marini",
        "Grasso",
        "Valentini",
        "Messina",
    ];
    let mut text = String::new();
    for index in 0..500usize {
        let first = firsts[index % firsts.len()];
        let last = lasts[(index / firsts.len()) % lasts.len()];
        text.push_str(&format!(
            "{first} {last} ha partecipato al progetto numero {index} con il gruppo di ricerca. \
             Nel 2009 il comitato ha approvato il piano e ha pubblicato i risultati. "
        ));
    }
    text
}

#[test]
#[ignore = "manual performance probe"]
fn large_scene_perf_probe() {
    let scenes = vec![SceneInput {
        scene_id: "perf".to_string(),
        text: synthetic_scene(),
    }];
    let request = ScriptRequest {
        language: "it".to_string(),
        scenes,
        editorial: None,
        entity_safety_cap: 0,
    };
    let started = Instant::now();
    let manifest = analyze_script(&request).expect("analysis");
    let elapsed = started.elapsed();
    let entities = manifest.scenes[0].entities.len();
    eprintln!(
        "PERF entities={entities} elapsed_ms={} words={}",
        elapsed.as_millis(),
        request.scenes[0].text.split_whitespace().count()
    );
}
