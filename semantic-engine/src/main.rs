fn main() -> std::io::Result<()> {
    // Resolve the bundled ONNX Runtime before the NDJSON worker pool starts.
    visualner::configure_ner_runtime();
    semantic_engine::protocol::run_stdio()
}
