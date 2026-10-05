fn main() {
    let mode = std::env::args().nth(1);
    let result = if mode.as_deref() == Some("phrase-impact")
        || std::env::args()
            .next()
            .as_deref()
            .is_some_and(|path| path.ends_with("phrase_impact"))
    {
        pipelinegen_muscles::phrase_impact_worker::run_stdio()
    } else {
        pipelinegen_muscles::run_stdio()
    };
    if let Err(error) = result {
        eprintln!("pipelinegen-muscles: {error}");
        std::process::exit(1);
    }
}
