use crate::error::MuscleError;

/// Sealed compiled audio plan shapes (compiled-audio-plan.v1/v2).
/// The protocol boundary holds these typed structs instead of untyped JSON:
/// a malformed plan fails request parsing fail-closed.
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Plan {
    #[serde(default)]
    pub(super) audio_plan_version: String,
    pub(super) duration_us: i64,
    // Frame-aligned pad target for the mastered file (audio_duration >=
    // video_duration). Absent/zero means "pad to duration_us".
    #[serde(default)]
    pub(super) master_duration_us: Option<i64>,
    #[serde(default)]
    pub(super) primary_events: Vec<Event>,
    #[serde(default)]
    pub(super) tracks: Vec<Track>,
    #[serde(default)]
    pub(super) background_music: Vec<Layer>,
    #[serde(default)]
    pub(super) sfx: Vec<Layer>,
    #[serde(default)]
    pub(super) automation: Vec<Automation>,
    pub(super) canonical_audio_profile: Output,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Track {
    pub(super) track_id: String,
    pub(super) role: String,
    #[serde(default)]
    pub(super) events: Vec<Event>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Event {
    #[serde(default)]
    pub(super) event_id: String,
    pub(super) r#type: String,
    pub(super) asset_id: Option<String>,
    pub(super) timeline_start_us: i64,
    pub(super) duration_us: i64,
    #[serde(default)]
    pub(super) source_in_us: i64,
    #[serde(default)]
    pub(super) source_duration_us: i64,
    #[serde(default)]
    pub(super) use_original_audio: bool,
    #[serde(default)]
    pub(super) gain_db: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Output {
    pub(super) codec: String,
    pub(super) profile: String,
    pub(super) sample_rate: i64,
    pub(super) channels: i64,
    pub(super) channel_layout: String,
    pub(super) bitrate: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Layer {
    pub(super) asset_id: String,
    pub(super) timeline_start_us: i64,
    pub(super) duration_us: i64,
    #[serde(default)]
    pub(super) gain_db: f64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Automation {
    #[serde(default)]
    pub(super) target_track_id: String,
    #[serde(default)]
    pub(super) target_layer: String,
    #[serde(default)]
    pub(super) trigger_track_id: String,
    pub(super) start_us: i64,
    pub(super) end_us: i64,
    pub(super) gain_db: f64,
    #[serde(default)]
    pub(super) attack_us: i64,
    #[serde(default)]
    pub(super) release_us: i64,
}

pub(super) fn validate_plan(plan: &Plan) -> Result<(), MuscleError> {
    if !plan.audio_plan_version.is_empty()
        && plan.audio_plan_version != "compiled-audio-plan.v1"
        && plan.audio_plan_version != "compiled-audio-plan.v2"
    {
        return Err("unsupported audio plan version".into());
    }
    if plan.audio_plan_version == "compiled-audio-plan.v2" && plan.tracks.is_empty() {
        return Err("compiled-audio-plan.v2 requires tracks".into());
    }
    if plan.duration_us <= 0
        || plan.canonical_audio_profile.codec != "aac"
        || plan.canonical_audio_profile.sample_rate != 48000
        || plan.canonical_audio_profile.channels != 2
        || plan.canonical_audio_profile.channel_layout != "stereo"
        || !plan
            .canonical_audio_profile
            .profile
            .eq_ignore_ascii_case("lc")
        || plan.canonical_audio_profile.bitrate.trim().is_empty()
    {
        return Err("audio_plan violates canonical AAC-LC stereo contract".into());
    }
    let use_tracks = !plan.tracks.is_empty();
    let mut expected = 0;
    let mut event_ids = std::collections::HashSet::new();
    for track in &plan.tracks {
        if track.track_id.trim().is_empty()
            || !["VOICEOVER", "CLIP_AUDIO", "BGM", "SFX"].contains(&track.role.as_str())
        {
            return Err("audio track id is required".into());
        }
        for event in &track.events {
            if event.event_id.trim().is_empty() || !event_ids.insert(event.event_id.clone()) {
                return Err("audio event id is missing or duplicated".into());
            }
            if (track.role == "VOICEOVER"
                && event.r#type != "VOICEOVER"
                && event.r#type != "SILENCE")
                || (track.role == "CLIP_AUDIO" && event.r#type != "CLIP_AUDIO")
                || (track.role == "BGM" && event.r#type != "BGM")
                || (track.role == "SFX" && event.r#type != "SFX")
            {
                return Err("audio event role does not match track".into());
            }
        }
    }
    for event in primary_events(plan) {
        if (!use_tracks && event.timeline_start_us != expected)
            || event.duration_us <= 0
            || event.timeline_start_us < 0
            || event.timeline_start_us + event.duration_us > plan.duration_us
        {
            return Err("audio_plan events are not contiguous and bounded by the timeline".into());
        }
        match event.r#type.as_str() {
            "SILENCE" => {}
            "VOICEOVER" => {
                if event.asset_id.as_deref().unwrap_or("").is_empty() {
                    return Err("audio event asset is unresolved".into());
                }
            }
            "CLIP_AUDIO" | "BGM" | "SFX" => {
                if event.asset_id.as_deref().unwrap_or("").is_empty() {
                    return Err("audio event asset is unresolved".into());
                }
                if event.r#type == "CLIP_AUDIO" && !event.use_original_audio {
                    return Err("CLIP_AUDIO must use the original clip audio".into());
                }
            }
            _ => return Err("audio_plan contains an unknown event type".into()),
        }
        if event.source_in_us < 0
            || event.source_duration_us < 0
            || ((event.r#type == "VOICEOVER"
                || event.r#type == "CLIP_AUDIO"
                || event.r#type == "BGM"
                || event.r#type == "SFX")
                && event.source_duration_us <= 0)
        {
            return Err("audio event source range is invalid".into());
        }
        expected += event.duration_us;
    }
    if !use_tracks && expected != plan.duration_us {
        return Err("audio_plan duration does not equal event end".into());
    }
    for layer in plan.background_music.iter().chain(plan.sfx.iter()) {
        if layer.asset_id.trim().is_empty()
            || layer.timeline_start_us < 0
            || layer.duration_us <= 0
            || layer.timeline_start_us + layer.duration_us > plan.duration_us
        {
            return Err("audio layer is outside the canonical timeline".into());
        }
    }
    for automation in &plan.automation {
        let target = if !automation.target_track_id.trim().is_empty() {
            automation.target_track_id.trim()
        } else {
            automation.target_layer.trim()
        };
        if target.is_empty()
            || automation.start_us < 0
            || automation.end_us <= automation.start_us
            || automation.end_us > plan.duration_us
            || automation.attack_us < 0
            || automation.release_us < 0
        {
            return Err("audio automation is outside the canonical timeline".into());
        }
    }
    Ok(())
}

fn primary_events(plan: &Plan) -> Vec<&Event> {
    if !plan.tracks.is_empty() {
        return plan
            .tracks
            .iter()
            .filter(|track| {
                ["VOICEOVER", "CLIP_AUDIO", "BGM", "SFX"].contains(&track.role.as_str())
            })
            .flat_map(|track| track.events.iter())
            .collect();
    }
    plan.primary_events.iter().collect()
}

pub(super) fn track_events(plan: &Plan) -> Vec<(String, &Event)> {
    if !plan.tracks.is_empty() {
        return plan
            .tracks
            .iter()
            .flat_map(|track| {
                track
                    .events
                    .iter()
                    .map(move |event| (track.track_id.clone(), event))
            })
            .collect();
    }
    plan.primary_events
        .iter()
        .map(|event| ("legacy-primary".to_string(), event))
        .collect()
}
