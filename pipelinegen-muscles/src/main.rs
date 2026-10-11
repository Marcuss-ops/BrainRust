fn is_binary_name(path: Option<&str>, name: &str) -> bool {
    path.is_some_and(|path| path.ends_with(name))
}

fn main() {
    let argv0 = std::env::args().next();
    let mode = std::env::args().nth(1);
    let result = if mode.as_deref() == Some("phrase-impact")
        || is_binary_name(argv0.as_deref(), "phrase_impact")
    {
        pipelinegen_muscles::phrase_impact_worker::run_stdio()
    } else if mode.as_deref() == Some("scene-intelligence")
        || is_binary_name(argv0.as_deref(), "scene_intelligence")
    {
        pipelinegen_muscles::scene_intelligence_worker::run_stdio()
    } else {
        pipelinegen_muscles::run_stdio()
    };
    if let Err(error) = result {
        eprintln!("pipelinegen-muscles: {error}");
        std::process::exit(1);
    }
}
