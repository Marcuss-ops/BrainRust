fn main() {
    if let Err(error) = pipelinegen_muscles::run_stdio() {
        eprintln!("pipelinegen-muscles: {error}");
        std::process::exit(1);
    }
}
