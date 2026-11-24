use super::*;

/// True when the sentence text contains one of the shared attribution cues
/// ("secondo", "according", "selon", ...). The list already exists in the
/// discourse lexicon, so source admission adds no second vocabulary.
pub(super) fn has_attribution_cue(sentence: &str) -> bool {
    let words: Vec<String> = sentence
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    semantic_core::lexicon::lexicon()
        .discourse
        .attribution
        .iter()
        .any(|cue| words.iter().any(|word| word == cue))
}

/// Resolves the speaker of a quote from the data the manifest already has: an
/// identity mention in the same sentence, nearest to the quote first, people
/// before organizations. No second NLP pass and no new extractor.
pub(super) fn quote_author<'a>(
    scene: &'a SceneManifest,
    quote: &semantic_core::Candidate,
) -> Option<&'a semantic_core::Candidate> {
    scene
        .entities
        .iter()
        .filter(|entity| entity.sentence_index == quote.sentence_index)
        .filter(|entity| {
            matches!(
                entity.kind.as_str(),
                "PERSON" | "ORG" | "ORGANIZATION" | "MEDIA_OUTLET"
            )
        })
        .filter(|entity| entity.start_byte != quote.start_byte || entity.end_byte != quote.end_byte)
        .min_by_key(|entity| {
            let kind_rank = if entity.kind == "PERSON" { 0 } else { 1 };
            let gap = if entity.end_byte <= quote.start_byte {
                quote.start_byte.saturating_sub(entity.end_byte)
            } else if entity.start_byte >= quote.end_byte {
                entity.start_byte.saturating_sub(quote.end_byte)
            } else {
                0
            };
            (kind_rank, gap)
        })
}

/// TICKET-46: one quote already grounded by `quote_candidates`, credited to the
/// speaker the same sentence names. An unattributed quote stays a plain quote:
/// inventing an author is worse than showing none.
pub(super) fn push_quote_attributions(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for quote in &scene.quotes {
            let Some(author) = quote_author(scene, quote) else {
                continue;
            };
            if !identities.contains_key(&author.id) {
                continue;
            }
            let Some(author_label) = ties(identities, &author.id) else {
                continue;
            };
            let role_surface = scene
                .person_roles
                .iter()
                .find(|role| {
                    role.person_id == author.id
                        && role.status == semantic_core::RoleStatus::Asserted
                })
                .map(|role| role.role_surface.clone());
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "quote_attribution",
                "quote_id": quote.id,
                "quote_text": quote.text,
                "author_id": author.id,
                "author_label": author_label,
                "author_type": author.kind,
                "role_surface": role_surface,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "quote_attribution",
                scene_id: scene.scene_id.as_str(),
                text,
                start: quote.start_byte,
                end: quote.end_byte,
                title: format!("{author_label} — {}", quote.text),
                suggested_visual: "quote_attribution",
                priority: 0.7,
                entity_ids: vec![author.id.clone()],
                covered_fact_ids: Vec::new(),
                payload,
                dedup_key: format!("quote_attribution:{}:{}", scene.scene_id, quote.id),
                rationale: "quote grounded in the source and credited to a same-sentence identity",
            }) {
                candidates.push(candidate);
            }
        }
    }
}

/// TICKET-46: a citable outlet the sentence attributes a claim to. The recipe
/// does not emit a new card: it pins the certified archival-source entrance on
/// the phrase or quote that names the outlet, so the existing item gains the
/// emphasis instead of a duplicate appearing beside it.
pub(super) fn push_source_highlights(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    texts: &HashMap<&str, &str>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for source in scene
            .entities
            .iter()
            .filter(|entity| entity.kind == semantic_core::entity_type_terms::MEDIA_OUTLET)
        {
            if !identities.contains_key(&source.id) {
                continue;
            }
            let Some(source_label) = ties(identities, &source.id) else {
                continue;
            };
            // Prefer a spoken quote in the same sentence; otherwise the sentence
            // phrase that carries the attribution cue.
            let highlight = scene
                .quotes
                .iter()
                .find(|quote| quote.sentence_index == source.sentence_index)
                .map(|quote| {
                    (
                        "quote",
                        quote.start_byte,
                        quote.end_byte,
                        quote.text.as_str(),
                        quote.evidence.as_str(),
                    )
                })
                .or_else(|| {
                    scene
                        .short_phrases
                        .iter()
                        .chain(scene.long_phrases.iter())
                        .find(|phrase| {
                            phrase.sentence_index == source.sentence_index
                                && has_attribution_cue(&phrase.evidence)
                        })
                        .map(|phrase| {
                            (
                                "phrase",
                                phrase.start_byte,
                                phrase.end_byte,
                                phrase.text.as_str(),
                                phrase.evidence.as_str(),
                            )
                        })
                });
            let Some((highlight_kind, start, end, highlight_text, sentence)) = highlight else {
                continue;
            };
            if !has_attribution_cue(sentence) {
                continue;
            }
            let payload = json!({
                "schema": RECIPE_SCHEMA,
                "recipe": "source_highlight",
                "source_id": source.id,
                "source_label": source_label,
                "source_surface": source.text,
                "highlight_kind": highlight_kind,
                "highlight_text": highlight_text,
            });
            if let Some(candidate) = candidate(RecipeCandidate {
                recipe: "source_highlight",
                scene_id: scene.scene_id.as_str(),
                text,
                start,
                end,
                title: format!("{source_label} — source"),
                suggested_visual: "source_highlight",
                priority: 0.74,
                entity_ids: vec![source.id.clone()],
                covered_fact_ids: Vec::new(),
                payload,
                dedup_key: format!("source_highlight:{}:{}", scene.scene_id, source.id),
                rationale: "attributed claim and a same-sentence media outlet",
            }) {
                candidates.push(candidate);
            }
        }
    }
}
