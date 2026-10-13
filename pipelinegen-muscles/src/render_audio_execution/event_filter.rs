pub(super) fn event_filter(
    index: usize,
    source_in_us: i64,
    source_end_us: i64,
    delay_ms: i64,
    gain: &str,
    eval_mode: &str,
    normalize: &str,
) -> String {
    format!(
        "[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{normalize}adelay={delay_ms}|{delay_ms},volume='{gain}':{eval_mode}[a{index}]",
        source_in_us as f64 / 1_000_000.0,
        source_end_us as f64 / 1_000_000.0
    )
}
