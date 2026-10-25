use super::*;

pub(crate) fn to_visual_proposal(
    candidate: VisualCandidate,
    source: &str,
    sentence_spans: &[semantic_core::sentences::SentenceSpan],
    scene_id: &str,
    index: usize,
) -> (VisualProposal, Option<SearchFeedbackEntry>) {
    let score = match candidate.kind {
        "chart" => 0.9,
        "timeline" => 0.8,
        "multiple_entities" => 0.7,
        _ => 0.6,
    };
    // The ladder widens on purpose: the exact identity first, then a bounded
    // source phrase, then the full sentence for an illustrative stock search.
    // Every rung is an exact source slice, so a provider hit is traceable. A
    // co-mention searches each name on its own, never their joined label: a
    // label such as "Sofia Rinaldi Marta Greco" does not occur in the source.
    let mut visual_query_suggestions: Vec<String> = Vec::new();
    let mut queries: Vec<SearchQuery> = Vec::new();
    if !candidate.entity_texts.is_empty() {
        for text in &candidate.entity_texts {
            if text.is_empty() || is_noise_query(text) {
                continue;
            }
            if !visual_query_suggestions.iter().any(|query| query == text) {
                visual_query_suggestions.push(text.clone());
            }
            queries.push(SearchQuery {
                kind: "exact_mention".to_string(),
                text: text.clone(),
            });
        }
        if candidate.kind == "entity_search" {
            let sentence = sentence_spans
                .iter()
                .find(|span| candidate.start >= span.start_byte && candidate.end <= span.end_byte);
            if let Some(sentence) = sentence {
                if let Some(context) =
                    contextual_visual_query(source, sentence, candidate.start, candidate.end)
                {
                    if !is_noise_query(context)
                        && !visual_query_suggestions.iter().any(|query| query == context)
                    {
                        visual_query_suggestions.push(context.to_string());
                        queries.push(SearchQuery {
                            kind: "context_phrase".to_string(),
                            text: context.to_string(),
                        });
                    }
                }
                let sentence = sentence.text(source);
                if !is_noise_query(sentence)
                    && !visual_query_suggestions
                        .iter()
                        .any(|query| query == sentence)
                {
                    visual_query_suggestions.push(sentence.to_string());
                    queries.push(SearchQuery {
                        kind: "source_sentence".to_string(),
                        text: sentence.to_string(),
                    });
                }
            }
        } else if !is_noise_query(&candidate.evidence)
            && !visual_query_suggestions
                .iter()
                .any(|query| query == &candidate.evidence)
        {
            visual_query_suggestions.push(candidate.evidence.clone());
            queries.push(SearchQuery {
                kind: "source_sentence".to_string(),
                text: candidate.evidence.clone(),
            });
        }
    }
    let proposal_id = format!("{scene_id}_visual_{:03}", index + 1);
    let feedback = candidate
        .requires_verified_images
        .then(|| SearchFeedbackEntry {
            proposal_id: proposal_id.clone(),
            scene_id: scene_id.to_string(),
            queries: queries.clone(),
            status: "pending".to_string(),
            retry: None,
        })
        .filter(|entry| !entry.queries.is_empty());
    let proposal = VisualProposal {
        id: proposal_id,
        proposal_type: candidate.kind.to_string(),
        entity_ids: candidate.entity_ids,
        visual_query_suggestions,
        suggested_visual: candidate
            .suggested_layout
            .unwrap_or(candidate.kind)
            .to_string(),
        status: "requires_semantic_validation".to_string(),
        score,
        chart_items: candidate.chart_items,
        temporal_surfaces: candidate.temporal_surfaces,
        evidence: candidate.evidence,
        start_byte: candidate.start,
        end_byte: candidate.end,
    };
    (proposal, feedback)
}

/// Noise surfaces never become search proposals. A curated filler phrase
/// ("non è così", "ancora oggi", "per questo") or an editorial heading is
/// discourse scaffolding, not a visual subject: its text stays available in
/// the scene inventory (phrase candidates), it just never becomes a query.
fn is_noise_query(text: &str) -> bool {
    let normalized = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    normalized.is_empty()
        || visualner::is_heading_surface(&normalized)
        || semantic_core::lexicon::lexicon()
            .stopwords
            .contains_filler_phrase(&normalized)
}

/// Returns a short, contiguous source phrase around an entity mention. This
/// gives image/stock providers a useful multiword concept while preserving an
/// exact source surface; the full canonical sentence remains available too.
pub(crate) fn contextual_visual_query<'a>(
    source: &'a str,
    sentence: &semantic_core::sentences::SentenceSpan,
    mention_start: usize,
    mention_end: usize,
) -> Option<&'a str> {
    // Six words is the shortest window that keeps a mention's subject and its
    // immediate action together while staying a provider-friendly query. A
    // multiword mention must not push the window past it (and end on a bare
    // article such as "la").
    const MAX_WORDS: usize = 6;
    const WORDS_BEFORE: usize = 2;
    const WORDS_AFTER: usize = 4;

    let text = sentence.text(source);
    let words: Vec<(usize, usize)> = text
        .split_whitespace()
        .map(|word| {
            let start = word.as_ptr() as usize - text.as_ptr() as usize;
            let end = start + word.len();
            (start + sentence.start_byte, end + sentence.start_byte)
        })
        .collect();
    let first = words
        .iter()
        .position(|(start, end)| *start < mention_end && mention_start < *end)?;
    let last = words
        .iter()
        .rposition(|(start, end)| *start < mention_end && mention_start < *end)?;
    let mut first = first.saturating_sub(WORDS_BEFORE);
    let mut after = (last + 1 + WORDS_AFTER).min(words.len());
    if after - first > MAX_WORDS {
        after = first + MAX_WORDS;
    }
    if last + 1 > after {
        after = last + 1;
        first = after.saturating_sub(MAX_WORDS);
    }
    let start = words.get(first)?.0;
    let end = words.get(after.checked_sub(1)?)?.1;
    source.get(start..end)
}

pub(crate) fn retain_grounded_entities(
    source: &str,
    entities: Vec<VisualEntity>,
) -> (Vec<VisualEntity>, usize) {
    let total = entities.len();
    // One grounding gate for every projection: the shared `retain_grounded`
    // owns the byte-span check, and entity surfaces additionally require their
    // display text to be the verbatim evidence slice.
    let (kept, _) = retain_grounded(
        source,
        entities,
        |entity| entity.start,
        |entity| entity.end,
        |entity| entity.evidence.as_str(),
    );
    let kept: Vec<VisualEntity> = kept
        .into_iter()
        .filter(|entity| entity.text == entity.evidence)
        .collect();
    let dropped = total.saturating_sub(kept.len());
    (kept, dropped)
}

pub(crate) fn retain_grounded_temporal(
    source: &str,
    facts: Vec<TemporalFact>,
) -> Vec<TemporalFact> {
    retain_grounded(
        source,
        facts,
        |fact| fact.start_byte,
        |fact| fact.end_byte,
        |fact| fact.evidence.as_str(),
    )
    .0
}

pub(crate) fn retain_grounded_candidates(
    source: &str,
    candidates: Vec<VisualCandidate>,
) -> (Vec<VisualCandidate>, usize) {
    retain_grounded(
        source,
        candidates,
        |candidate| candidate.start,
        |candidate| candidate.end,
        |candidate| candidate.evidence.as_str(),
    )
}
