use imagecatalog::{plan, ImageCatalogRequest, PLAN_SCHEMA, REQUEST_SCHEMA};
use ndjson_stdio::{check_envelope, ErrorCode, WorkerError};

fn error_response(error: &WorkerError) -> String {
    error.log();
    serde_json::json!({
        "schema_version": PLAN_SCHEMA,
        "error": error.message,
        "error_code": error.code,
    })
    .to_string()
}

fn main() {
    if let Err(error) = ndjson_stdio::run_stdio(process_line) {
        eprintln!("imagecatalog read: {error}");
        std::process::exit(1);
    }
}

/// Maps one request line to one plan line. A malformed line is reported as an
/// error response, never by exiting: PipelineGen drives this as a persistent
/// worker.
fn process_line(line: &str) -> String {
    let request: ImageCatalogRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(&WorkerError::new(
                ErrorCode::Decode,
                format!("imagecatalog decode: {error}"),
            ))
        }
    };
    if let Err(error) = check_envelope(
        "imagecatalog",
        request.version.as_deref(),
        REQUEST_SCHEMA,
        request.operation.as_deref(),
        "plan",
    ) {
        return error_response(&error);
    }
    match plan(&request) {
        Ok(plan) => match serde_json::to_string(&plan) {
            Ok(line) => line,
            Err(error) => error_response(&WorkerError::new(
                ErrorCode::Encode,
                format!("imagecatalog encode: {error}"),
            )),
        },
        Err(message) => error_response(&WorkerError::new(ErrorCode::Execution, message)),
    }
}
