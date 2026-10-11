use super::{checked_multi_inputs, finish_run, write_concat_list, ExpectedInputs};
use crate::artifact::{failed_response, part_path};
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;

pub(super) fn execute(request: Request) -> Response {
    let (inputs, output) = match checked_multi_inputs(
        &request,
        ExpectedInputs::AtLeastOne,
        "input_paths are required",
    ) {
        Ok(values) => values,
        Err(response) => return *response,
    };
    let list_path = match write_concat_list(&inputs, "concat") {
        Ok(path) => path,
        Err(error) => {
            return failed_response(Some(inputs[0].clone()), error);
        }
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let part = part_path(&output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]);
    command.arg(list_path.to_string_lossy().to_string());
    command.args(["-c", "copy", &part]);
    let result = command.output();
    let _ = fs::remove_file(&list_path);
    finish_run("merge_inputs", &inputs[0], &part, &output, result, None)
}
