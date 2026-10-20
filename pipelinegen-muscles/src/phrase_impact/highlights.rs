//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

pub(super) fn build_scene_highlights(
    transcript: &str,
    global_segments: &[Segment],
    global_importance: &[f64],
    scenes: &[SceneInput],
    language: &str,
    stopwords: &HashSet<String>,
) -> (Vec<SceneHighlight>, bool) {
    let mut out = Vec::new();
    let mut certified = true;
    let mut seen_ids = std::collections::HashSet::new();
    // A sentence the model repeats verbatim in every scene (the mandated
    // opening line) is document material: it must be published once, not
    // once per scene.
    let mut used_bullet_texts: HashSet<String> = HashSet::new();
    for scene in scenes {
        if scene.scene_id.trim().is_empty() || !seen_ids.insert(scene.scene_id.clone()) {
            certified = false;
            continue;
        }
        let globally_indexed;
        let scene_segments: Vec<Segment>;
        let local_importance: Vec<f64>;
        if let (Some(s), Some(e)) = (scene.start_byte, scene.end_byte) {
            let valid = e <= transcript.len()
                && s < e
                && transcript.is_char_boundary(s)
                && transcript.is_char_boundary(e)
                && transcript[s..e] == *scene.text;
            if !valid {
                // Slice mismatch: local-only analysis, no fabricated globals.
                globally_indexed = false;
                let parts = split_sentences(&scene.text, language);
                if parts.is_empty() {
                    out.push(SceneHighlight {
                        scene_id: scene.scene_id.clone(),
                        title: None,
                        title_status: "unavailable".into(),
                        title_source: "none".into(),
                        bullets: Vec::new(),
                        highlights: Vec::new(),
                        chart_values: scene_chart_values(scene),
                        globally_indexed,
                    });
                    continue;
                }
                // Map local sentences onto overlapping global range when the
                // raw bytes match somewhere (repeated text => first match is
                // NOT assumed; require the declared slice, else local-only).
                certified = false;
                scene_segments = parts;
                local_importance = vec![0.5; scene_segments.len()];
            } else {
                globally_indexed = true;
                let mut v = Vec::new();
                let mut imp = Vec::new();
                for (gi, seg) in global_segments.iter().enumerate() {
                    if seg.start_byte >= s && seg.end_byte <= e {
                        v.push(seg.clone());
                        imp.push(*global_importance.get(gi).unwrap_or(&0.5));
                    }
                }
                if v.is_empty() {
                    let parts = split_sentences(&scene.text, language);
                    scene_segments = parts;
                    local_importance = vec![0.5; scene_segments.len()];
                } else {
                    scene_segments = v;
                    local_importance = imp;
                }
            }
        } else {
            globally_indexed = false;
            let parts = split_sentences(&scene.text, language);
            if parts.is_empty() {
                out.push(SceneHighlight {
                    scene_id: scene.scene_id.clone(),
                    title: None,
                    title_status: "unavailable".into(),
                    title_source: "none".into(),
                    bullets: Vec::new(),
                    highlights: Vec::new(),
                    chart_values: scene_chart_values(scene),
                    globally_indexed,
                });
                continue;
            }
            scene_segments = parts;
            local_importance = vec![0.5; scene_segments.len()];
        }
        if scene_segments.is_empty() {
            out.push(SceneHighlight {
                scene_id: scene.scene_id.clone(),
                title: None,
                title_status: "unavailable".into(),
                title_source: "none".into(),
                bullets: Vec::new(),
                highlights: Vec::new(),
                chart_values: scene_chart_values(scene),
                globally_indexed,
            });
            continue;
        }
        let (title, title_status, title_source) =
            select_scene_title(&scene.text, scene.topic.as_deref(), stopwords);
        // Bullet budget scales with scene length; short scenes get none invented.
        let words: usize = scene.text.split_whitespace().count();
        let bullet_n = if words < 15 || scene_segments.is_empty() {
            0
        } else if words < 60 {
            1.min(scene_segments.len())
        } else {
            3.min(scene_segments.len())
        };
        let mut order: Vec<usize> = (0..scene_segments.len()).collect();
        order.sort_by(|&a, &b| {
            local_importance[b]
                .total_cmp(&local_importance[a])
                .then(a.cmp(&b))
        });
        let scene_texts: Vec<&str> = scene_segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        let mut bullets = Vec::new();
        for &li in &order {
            if bullets.len() >= bullet_n {
                break;
            }
            // A bullet must OPEN a sentence: the continuation chunk of a
            // sentence longer than MAX_SEGMENT_WORDS is not one.
            if !bullet_starts_sentence(&scene_texts, li) {
                continue;
            }
            // Rejoin the chunks of a long sentence so a bullet is never a
            // truncated mid-sentence slice ("...con la tutela dei diritti di").
            let mut end = li + 1;
            while end < scene_segments.len()
                && !bullet_closes_sentence(&scene_segments[end - 1].text)
            {
                end += 1;
            }
            let text = scene_segments[li..end]
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            // Fragments are not bullets: "impronta 33." is noise, not a point.
            if !bullet_has_substance(&text) {
                continue;
            }
            // A sentence repeated verbatim across scenes is published once.
            // Only verified (globally indexed) scenes participate: a local-only
            // scene cannot prove the shared identity, so it keeps its own
            // extractive bullets.
            if globally_indexed && !used_bullet_texts.insert(bullet_identity(&text)) {
                continue;
            }
            // Global sentence indices when indexed, else local indices.
            let (gs, ge) = if globally_indexed {
                let gs = global_segments
                    .iter()
                    .position(|g| g.start_byte == scene_segments[li].start_byte)
                    .unwrap_or(li);
                let ge = global_segments
                    .iter()
                    .position(|g| g.start_byte == scene_segments[end - 1].start_byte)
                    .map(|index| index + 1)
                    .unwrap_or(end);
                (gs, ge)
            } else {
                (li, end)
            };
            bullets.push(SceneBulletSpan {
                sentence_start: gs,
                sentence_end: ge,
                text,
            });
        }
        bullets.sort_by_key(|a| a.sentence_start);
        let highlights = select_phrase_spans(&scene_segments, &local_importance, stopwords, 2);
        out.push(SceneHighlight {
            scene_id: scene.scene_id.clone(),
            title,
            title_status,
            title_source,
            bullets,
            highlights,
            chart_values: scene_chart_values(scene),
            globally_indexed,
        });
    }
    (out, certified)
}
