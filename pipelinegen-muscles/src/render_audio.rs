#[path = "render_audio_execution.rs"]
mod execution;
#[path = "render_audio_plan.rs"]
mod plan;
#[path = "render_audio_probe.rs"]
mod probe;

#[derive(serde::Deserialize)]
struct Plan {
    #[serde(default)]
    audio_plan_version: String,
    duration_us: i64,
    // Frame-aligned pad target for the mastered file (audio_duration >=
    // video_duration). Absent/zero means "pad to duration_us".
    #[serde(default)]
    master_duration_us: Option<i64>,
    #[serde(default)]
    primary_events: Vec<Event>,
    #[serde(default)]
    tracks: Vec<Track>,
    #[serde(default)]
    background_music: Vec<Layer>,
    #[serde(default)]
    sfx: Vec<Layer>,
    #[serde(default)]
    automation: Vec<Automation>,
    canonical_audio_profile: Output,
}

#[derive(serde::Deserialize)]
struct Track {
    track_id: String,
    role: String,
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(serde::Deserialize)]
struct Event {
    #[serde(default)]
    event_id: String,
    r#type: String,
    asset_id: Option<String>,
    timeline_start_us: i64,
    duration_us: i64,
    #[serde(default)]
    source_in_us: i64,
    #[serde(default)]
    source_duration_us: i64,
    #[serde(default)]
    use_original_audio: bool,
    #[serde(default)]
    gain_db: f64,
}

#[derive(serde::Deserialize)]
struct Output {
    codec: String,
    profile: String,
    sample_rate: i64,
    channels: i64,
    channel_layout: String,
    bitrate: String,
}

#[derive(serde::Deserialize)]
struct Layer {
    asset_id: String,
    timeline_start_us: i64,
    duration_us: i64,
    #[serde(default)]
    gain_db: f64,
}

#[derive(serde::Deserialize)]
struct Automation {
    #[serde(default)]
    target_track_id: String,
    #[serde(default)]
    target_layer: String,
    #[serde(default)]
    trigger_track_id: String,
    start_us: i64,
    end_us: i64,
    gain_db: f64,
    #[serde(default)]
    attack_us: i64,
    #[serde(default)]
    release_us: i64,
}

pub(crate) fn execute(request: crate::protocol::Request) -> crate::protocol::Response {
    execution::execute(request)
}
