#[path = "render_audio_execution/envelopes.rs"]
mod envelopes;
#[path = "render_audio_execution/event_filter.rs"]
mod event_filter;
#[path = "render_audio_execution/loudness.rs"]
mod loudness;
#[path = "render_audio_execution/loudness_cache.rs"]
mod loudness_cache;
#[path = "render_audio_execution/sources.rs"]
mod sources;
use envelopes::*;
use event_filter::*;
use loudness::*;
use loudness_cache::LoudnessCache;
use sources::SourceCaches;

use crate::render_audio_plan::{track_events, validate_plan};

use super::probe::{probe_audio, probe_source, SourceProbe};
use crate::artifact::{part_path, publish_output};
use crate::hash::sha256_file;
use crate::process::{wait_child_with_deadline, FFmpegRunner};
use crate::protocol::{AudioAsset, Operation, Request, Response};
use crate::render_audio_plan::{Automation, Plan};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

// The plan carries gains in dB, while the FFmpeg volume filter treats a plain
// number as a linear multiplier (volume=0 silences the stream, and negative
// "gains" would invert/amplify instead of attenuating). Convert at the
// boundary with the standard power ratio so 0 dB maps to unity and the
// canonical duck levels (-12/-18/-24 dB) attenuate as the mix policy intends.
fn source_normalize(probe: &SourceProbe) -> &'static str {
    static CONVERSION: OnceLock<String> = OnceLock::new();
    if probe.sample_rate == u32::try_from(crate::config::CANONICAL_SAMPLE_RATE).ok()
        && probe.channels == Some(2)
        && probe.channel_layout.as_deref() == Some("stereo")
    {
        ""
    } else {
        CONVERSION.get_or_init(|| {
            format!(
                "aresample={},aformat=channel_layouts=stereo,",
                crate::config::CANONICAL_SAMPLE_RATE
            )
        })
    }
}

// ── Parallel stem fan-out: the only render path ────────────────────────────
//
// The audio master is rendered by splitting the compiled events into K
// independent stems and rendering them CONCURRENTLY, then summing the stems
// and encoding the master in one final AAC pass:
//
//   chains (one per compiled event/layer) ──round-robin──▶ stem 1..K
//   each stem: its own ffmpeg (event chains + silence bed + amix + pad)
//              → raw float PCM (f32le) streamed into a FIFO
//   final:     amix(stems) + pad/trim + alimiter → AAC-LC master (ONE encode)
//
// Why this shape:
// - FFmpeg serializes audio filters within one process: the old monolithic
//   graph used ~1 core and paid (decode+filter) + (encode) sequentially
//   inside one filter graph (~21-25 s for a 3-minute timeline on the bench
//   host). The fan-out spreads the mix across K processes (~2.6x wall on the
//   24-core bench host) and leaves the AAC encode as the serial floor.
// - amix with normalize=0 is plain summation, and float addition of the
//   same terms in a different grouping changes nothing audible (verified
//   level-identical against the old graph: mean/max volume identical), so
//   the grouping is free to follow the hardware.
// - Stems are pcm_f32le (lossless): the intermediate never quantizes, so
//   the fan-out cannot degrade the mix relative to a single float graph.
//
// The mix is only ever summed, never limited, per stem: the alimiter runs
// once on the TOTAL mix exactly like the old single graph, so limiter
// behaviour is unchanged. The silence bed is repeated per stem (a stem
// whose sources end early must still emit the full master length); N copies
// of silence sum to silence. Each stem pins its output to
// master_duration_us, and the final pass pins it again, so the master is
// frame-aligned exactly as before.
//
// K defaults to 4 (measured optimum on the 24-core bench host: more stems
// add FIFO streams without cutting the encode floor) and is clamped to
// the chain count; PIPELINE_MUSCLES_AUDIO_STEMS overrides it per host.
//
// The event chain builder below is the boundary where "Go decides, Rust
// executes" is enforced: every chain plays EXACTLY the declared source
// range [source_in_us, source_end) once, delayed to the declared timeline
// start. There is no loop filter, no stream_loop, and no duration discovery
// — a plan that wants the music repeated must contain N explicit events (the
// Go loop expander's output). If the source is shorter than the declared
// range, ensure_source fails the render instead of inventing a loop.
//
// eval_mode is the volume filter's expression-evaluation mode
// (volume_eval_mode): "eval=frame" when the gain expression depends on
// absolute output time (automation), "eval=once" for static gains.
//
// normalize is the source_normalize output for this event's asset: the
// aresample/aformat chain when the source is not canonical, empty when it
// already is 48 kHz stereo.

/// One compiled event/layer's fully-resolved filter inputs. The chain string
/// itself is rendered per stem (with stem-local input indices), so the graph
/// pieces live here instead of in a pre-built monolithic graph.
#[derive(Clone)]
struct ChainSpec {
    path: String,
    source_in_us: i64,
    source_end_us: i64,
    delay_ms: i64,
    gain: String,
    eval_mode: &'static str,
    normalize: String,
}

const STEM_FANOUT: usize = 4;

fn stem_count(chain_count: usize) -> usize {
    let configured = std::env::var("PIPELINE_MUSCLES_AUDIO_STEMS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    stem_count_from(chain_count, configured)
}

/// The pure decision: clamp the configured fan-out (default STEM_FANOUT) to
/// the chain count. Split from the env lookup so tests pin the semantics
/// without depending on the host environment.
fn stem_count_from(chain_count: usize, configured: Option<usize>) -> usize {
    if chain_count == 0 {
        return 0;
    }
    configured
        .filter(|value| *value >= 1)
        .unwrap_or(STEM_FANOUT)
        .min(chain_count)
}

/// One stem's render job: where the graph lives, the FIFO its raw float PCM
/// streams into, and which sources its argv must open.
#[derive(Clone)]
struct StemJob {
    script: String,
    sink: String,
    filter: String,
    paths: Vec<String>,
}

/// Round-robin over the chain order: heterogeneous chains (long voiceover
/// decodes vs short SFX) spread evenly across the stems, and the partition
/// is deterministic for a given plan.
fn partition_stems(chains: Vec<ChainSpec>, count: usize) -> Vec<Vec<ChainSpec>> {
    let mut stems: Vec<Vec<ChainSpec>> = (0..count.max(1)).map(|_| Vec::new()).collect();
    let buckets = stems.len();
    for (index, chain) in chains.into_iter().enumerate() {
        stems[index % buckets].push(chain);
    }
    stems.truncate(count);
    stems
}

fn silence_bed_filter(master_secs: f64) -> String {
    format!(
        "anullsrc=r={}:cl=stereo,atrim=duration={master_secs},asetpts=PTS-STARTPTS[asilence]",
        crate::config::CANONICAL_SAMPLE_RATE
    )
}

/// One stem's complete graph: its event chains (stem-local input indices),
/// its own silence bed, a plain sum, pinned to the master duration. No
/// limiter here: the limiter must see the TOTAL mix, once, in the final pass.
fn stem_filter_script(chains: &[ChainSpec], master_secs: f64) -> String {
    let mut parts: Vec<String> = chains
        .iter()
        .enumerate()
        .map(|(index, chain)| {
            event_filter(
                index,
                chain.source_in_us,
                chain.source_end_us,
                chain.delay_ms,
                &chain.gain,
                chain.eval_mode,
                &chain.normalize,
            )
        })
        .collect();
    parts.push(silence_bed_filter(master_secs));
    let labels: String = (0..chains.len())
        .map(|index| format!("[a{index}]"))
        .chain(std::iter::once("[asilence]".to_string()))
        .collect();
    parts.push(format!(
        "{labels}amix=inputs={}:duration=longest:normalize=0[mixed];\
         [mixed]apad=whole_dur={master_secs},atrim=duration={master_secs}[astem]",
        chains.len() + 1
    ));
    parts.join(";")
}
/// The final pass graph: sum the stems (or emit pure silence when the plan
/// has no audible events), pin to the master duration, and limit the TOTAL
/// mix exactly once.
///
/// The `aformat=sample_fmts=s16` at the encode boundary is a measured
/// performance requirement, not a quality preference: FFmpeg 4.4's generic
/// float/int32 → encoder conversion costs 2.3x its specialized s16 path
/// (15.2 s vs 6.5 s encoding the SAME 3-minute mix on the bench host — the
/// anomaly that made the old single-pass graph slower than its own parts).
/// The stems stay lossless float; the mix is rounded to 16-bit exactly
/// where the AAC codec will bury it (~-96 dB quantization floor under the
/// codec's own noise at 128 kbit/s).
fn final_mix_script(stem_count: usize, master_secs: f64) -> String {
    let pinned = format!(
        "apad=whole_dur={master_secs},atrim=duration={master_secs},alimiter=limit=0.95,aformat=sample_fmts=s16[aout]"
    );
    match stem_count {
        0 => format!("{};[asilence]{pinned}", silence_bed_filter(master_secs)),
        1 => format!("[0:a]{pinned}"),
        count => {
            let labels: String = (0..count).map(|index| format!("[{index}:a]")).collect();
            format!(
                "{labels}amix=inputs={count}:duration=longest:normalize=0[mixed];[mixed]{pinned}"
            )
        }
    }
}

fn remove_files(paths: &[String]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

/// Create the FIFO a stem streams into. `mkfifo` is a coreutils binary on
/// every Linux host this worker runs on, and the workspace denies `unsafe`,
/// so there is no in-process libc call to make here.
fn make_fifo(path: &str) -> Result<(), String> {
    let status = std::process::Command::new("mkfifo")
        .args(["-m", "600", path])
        .status()
        .map_err(|error| format!("create audio fifo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("create audio fifo: mkfifo failed for {path}"))
    }
}

/// A failed stage. The renderer signals failure with the two facts that
/// matter — the message and the optional source path — instead of building the
/// wire `Response` in every branch: `Response` is ~800 bytes, so returning it
/// in `Err` on every fallible stage costs more than the error path is worth.
/// One conversion happens at the boundary.
struct RenderError {
    source_path: Option<String>,
    message: String,
}

impl RenderError {
    fn new(source_path: Option<String>, message: impl ToString) -> Self {
        Self {
            source_path,
            message: message.to_string(),
        }
    }

    fn into_response(self) -> Response {
        Response::failure(Operation::RenderAudioPlan, self.source_path, self.message)
    }
}

/// Everything one render needs from the request, resolved once so the stages
/// below take `&self` instead of threading six locals through every branch.
struct RenderContext<'a> {
    output: &'a str,
    plan: &'a Plan,
    master_secs: f64,
    by_id: &'a HashMap<String, String>,
    ffmpeg: &'a str,
}

/// Files this render owns: filter scripts and the per-stem FIFOs. Every path
/// is deleted on drop, success included — the explicit `remove_files` calls
/// this replaces were one forgotten return away from leaking a FIFO that the
/// next render would then try to reuse.
#[derive(Default)]
struct Scratch {
    paths: Vec<String>,
}

impl Scratch {
    fn track(&mut self, path: String) {
        self.paths.push(path);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        remove_files(&self.paths);
    }
}

/// The running encode: one final-pass encoder reading every stem FIFO, plus
/// one writer process per stem. The encoder is always spawned first: it opens
/// the FIFOs (blocking until each writer opens its end), so the other order
/// deadlocks.
struct EncodeRun {
    encode: std::process::Child,
    writers: Vec<std::process::Child>,
    encode_started: std::time::Instant,
    mix_started: std::time::Instant,
}

/// The pre-mix wall, split so the BGM loudness measurement is not folded into
/// the source probes and plan setup. Both are reported to the caller: before
/// them, a render of a 14 s timeline could report 24 s of wall with only the
/// 1-2 s mix accounted, and the remainder had no owner.
struct PrepareTiming {
    prepare_ms: i64,
    loudness_ms: i64,
}

/// Per-stage timings plus the two failure modes that only exist once the
/// children are reaped: the encoder's own exit and the first writer failure.
struct EncodeTiming {
    encode: Result<(), String>,
    writer: Option<String>,
    aac_encode_ms: i64,
    mix_ms: i64,
}

fn kill_children(encode: &mut std::process::Child, writers: &mut [std::process::Child]) {
    let _ = encode.kill();
    for writer in writers {
        let _ = writer.kill();
    }
}

#[path = "render_audio_execution/render_context.rs"]
mod render_context;
impl EncodeRun {
    fn start(
        context: &RenderContext<'_>,
        jobs: &[StemJob],
        stem_inputs: &[String],
        filter_script: &str,
        part: &str,
        mix_started: std::time::Instant,
    ) -> Result<Self, RenderError> {
        // ── Final pass: sum the stems and encode the master — ONE AAC encode ──
        let canonical_rate = crate::config::CANONICAL_SAMPLE_RATE.to_string();
        let mut encode_command = FFmpegRunner::from_ffmpeg_path(context.ffmpeg).ffmpeg();
        encode_command.args(["-hide_banner", "-loglevel", "error", "-y"]);
        for input in stem_inputs {
            encode_command.args([
                "-f",
                "f32le",
                "-ar",
                canonical_rate.as_str(),
                "-ac",
                "2",
                "-i",
                input.as_str(),
            ]);
        }
        encode_command.args([
            "-filter_complex_script",
            filter_script,
            "-map",
            "[aout]",
            "-c:a",
            "aac",
            "-profile:a",
            "aac_low",
            "-ar",
            canonical_rate.as_str(),
            "-ac",
            "2",
            "-b:a",
            &context.plan.canonical_audio_profile.bitrate,
            "-t",
            &context.master_secs.to_string(),
            part,
        ]);
        let mut encode_command = encode_command.into_command();
        encode_command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let mut encode = encode_command.spawn().map_err(|error| {
            RenderError::new(
                None,
                format!("render_audio_plan encode failed to start: {error}"),
            )
        })?;
        let encode_started = std::time::Instant::now();

        let mut writers: Vec<std::process::Child> = Vec::with_capacity(jobs.len());
        for job in jobs {
            if let Err(error) = fs::write(&job.script, &job.filter) {
                kill_children(&mut encode, &mut writers);
                return Err(RenderError::new(
                    None,
                    format!("write audio filter script: {error}"),
                ));
            }
            let mut command = FFmpegRunner::from_ffmpeg_path(context.ffmpeg).ffmpeg();
            command.args(["-hide_banner", "-loglevel", "error", "-y"]);
            for input in &job.paths {
                command.args(["-i", input]);
            }
            command.args([
                "-filter_complex_script",
                &job.script,
                "-map",
                "[astem]",
                "-c:a",
                "pcm_f32le",
                "-ar",
                canonical_rate.as_str(),
                "-ac",
                "2",
                "-f",
                "f32le",
                job.sink.as_str(),
            ]);
            let mut command = command.into_command();
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            match command.spawn() {
                Ok(child) => writers.push(child),
                Err(error) => {
                    kill_children(&mut encode, &mut writers);
                    return Err(RenderError::new(
                        None,
                        format!("render_audio_plan stem render failed to start: {error}"),
                    ));
                }
            }
        }
        Ok(Self {
            encode,
            writers,
            encode_started,
            mix_started,
        })
    }

    /// Reap the encoder and then every writer. The encode deadline is a safety
    /// net, not a budget: the encoder drains the FIFOs at ~34x realtime.
    fn finish(self, master_secs: f64) -> EncodeTiming {
        let Self {
            encode,
            writers,
            encode_started,
            mix_started,
        } = self;
        let encode_deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(master_secs.max(0.0) as u64 / 10 + 180);
        let encode_output =
            wait_child_with_deadline(encode, encode_deadline, "render_audio_plan encode");
        let aac_encode_ms = encode_started.elapsed().as_millis() as i64;
        // With the encoder gone a producer can only be blocked in open(): 30s
        // covers the normal close path and bounds the pathological one.
        let writer_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut writer_error: Option<String> = None;
        for writer in writers {
            match wait_child_with_deadline(writer, writer_deadline, "render_audio_plan stem render")
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    if writer_error.is_none() {
                        writer_error = Some(format!(
                            "render_audio_plan stem render failed: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                }
                Err(error) => {
                    if writer_error.is_none() {
                        writer_error = Some(error.to_string());
                    }
                }
            }
        }
        let mix_ms = mix_started.elapsed().as_millis() as i64;
        let encode = match encode_output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "render_audio_plan encode failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => Err(error.to_string()),
        };
        EncodeTiming {
            encode,
            writer: writer_error,
            aac_encode_ms,
            mix_ms,
        }
    }
}

/// Render the compiled audio plan into one AAC-LC master.
///
/// The stages are separate so each one is readable on its own; the ordering
/// between them is load-bearing and stays visible here: resolve every source
/// (fail closed) → create the FIFOs → write the final graph → start the
/// encoder → start the writers → reap → probe/publish/hash.
pub(super) fn execute(request: Request) -> Response {
    let Request {
        output_path,
        audio_plan,
        audio_assets,
        ffmpeg_path,
        ..
    } = request;
    let Some(output) = output_path.filter(|value| !value.is_empty()) else {
        return RenderError::new(None, "output_path is required".to_string()).into_response();
    };
    // The plan arrives typed from the protocol boundary; a malformed plan
    // already failed request parsing fail-closed, so absence is the only
    // rejection left here.
    let Some(plan) = audio_plan else {
        return RenderError::new(None, "audio_plan is invalid".to_string()).into_response();
    };
    if let Err(error) = validate_plan(&plan) {
        return RenderError::new(None, error.to_string()).into_response();
    }
    let by_id: HashMap<String, String> = audio_assets
        .unwrap_or_default()
        .into_iter()
        .map(|asset: AudioAsset| (asset.asset_id, asset.path))
        .collect();
    // The plan carries the canonical microsecond duration, but the video
    // renderer emits FrameAt(duration_us) whole frames, which can round the
    // timeline up by up to half a frame. The master is padded to the
    // frame-aligned duration (master_duration_us) so audio_duration >=
    // video_duration instead of leaving a silent tail at the end of the mux.
    let context = RenderContext {
        output: &output,
        plan: &plan,
        master_secs: plan.master_duration_us.unwrap_or(plan.duration_us) as f64 / 1_000_000.0,
        by_id: &by_id,
        ffmpeg: ffmpeg_path.as_deref().unwrap_or("ffmpeg"),
    };
    context
        .render()
        .unwrap_or_else(|error| error.into_response())
}

#[cfg(test)]
#[path = "render_audio_execution_tests.rs"]
mod tests;
