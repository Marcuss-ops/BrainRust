#[cfg(test)]
mod tests {
    use crate::protocol::{Request, Response};
    use serde::Deserialize;
    use std::fs;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize)]
    struct Fixture {
        request: Request,
        response: Response,
    }

    fn fixture(name: &str) -> Fixture {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("mediaexec")
            .join("v1")
            .join(format!("{name}.json"));
        let data = fs::read(&path)
            .unwrap_or_else(|error| panic!("read shared fixture {}: {error}", path.display()));
        serde_json::from_slice(&data)
            .unwrap_or_else(|error| panic!("decode shared fixture {}: {error}", path.display()))
    }

    #[test]
    fn mediaexec_v1_shared_goldens_match_rust_wire_types() {
        for name in ["probe", "cut_batch", "render_stock", "normalize"] {
            let fixture = fixture(name);
            assert_eq!(fixture.request.version, crate::protocol::PROTOCOL_VERSION);
            fixture
                .request
                .validate()
                .unwrap_or_else(|error| panic!("fixture {name} failed validation: {error}"));
            assert_eq!(fixture.request.operation.as_str(), name);
            assert!(fixture.response.ok);
            assert_eq!(fixture.response.operation, name);

            match name {
                "probe" => {
                    assert!(fixture.request.source_path.is_some());
                    assert!(fixture.response.metadata.is_some());
                }
                "cut_batch" => {
                    let jobs = fixture.request.jobs.as_ref().expect("cut jobs");
                    assert_eq!(jobs.len(), 1);
                    assert_eq!(fixture.response.items.len(), 1);
                    assert_eq!(jobs[0].job_id, fixture.response.items[0].job_id);
                }
                "render_stock" => {
                    assert_eq!(fixture.request.input_paths.as_ref().unwrap().len(), 2);
                    assert_eq!(fixture.request.transitions.as_ref().unwrap().len(), 1);
                    assert_eq!(fixture.request.effect_paths.as_ref().unwrap().len(), 1);
                }
                "normalize" => {
                    assert!(fixture.request.source_path.is_some());
                    assert!(fixture.request.output_path.is_some());
                    assert_eq!(fixture.request.keep_audio, Some(true));
                }
                _ => unreachable!(),
            }
        }
    }
}
