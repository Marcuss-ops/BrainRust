use super::*;

/// Links identities with an additive prior only when a resolved discourse
/// thread provides an unambiguous sense from another mention in that thread.
pub fn link_all_with_threads(
    language: &str,
    scenes: &[(&str, &str)],
    identities: &[CanonicalIdentity],
    threads: &[crate::narrative::EntityThread],
) -> Vec<LinkedEntity> {
    let index = entity_index();
    identities
        .iter()
        .map(|identity| {
            let discourse_prior = resolved_thread_sense(language, identity, scenes, threads, index);
            // Context is the sentence around each grounded mention, never the
            // whole script: an actor mentioned in the next sentence must not
            // penalize the basketball player's identity. It is used only for
            // deterministic disambiguation and never copied into a query.
            let mut context = String::new();
            for mention in &identity.mentions {
                if let Some((_, text)) = scenes
                    .iter()
                    .find(|(scene_id, _)| *scene_id == mention.scene_id)
                {
                    context.push_str(sentence_window(text, mention.start_byte, mention.end_byte));
                    context.push(' ');
                }
            }
            let mut scored: Vec<(i32, &EntityIndexEntry)> = index
                .candidates(&identity.canonical_surface)
                .into_iter()
                .map(|entry| {
                    let prior = discourse_prior
                        .as_ref()
                        .is_some_and(|prior| prior.candidate_id == entry.id);
                    (
                        context_score(language, entry, &identity.kind, &context)
                            + if prior {
                                link_weights().discourse_prior_weight
                            } else {
                                0
                            },
                        entry,
                    )
                })
                .collect();
            scored.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.id.cmp(&right.1.id))
            });

            let link_candidates: Vec<LinkCandidate> = scored
                .iter()
                .take(link_weights().max_candidates)
                .map(|(score, entry)| {
                    let prior = discourse_prior
                        .as_ref()
                        .filter(|prior| prior.candidate_id == entry.id);
                    explain_candidate(*score, entry, language, identity, scenes, prior)
                })
                .collect();
            let link_margin = scored
                .get(1)
                .map(|second| scored[0].0.saturating_sub(second.0));

            let (status, entry) = match scored.as_slice() {
                [] => (LinkStatus::Unlinked, None),
                [(score, entry)] => {
                    if *score < link_weights().single_candidate_min_score {
                        (LinkStatus::Ambiguous, None)
                    } else {
                        (LinkStatus::Linked, Some(*entry))
                    }
                }
                [(best, entry), (second, _), ..] => {
                    // A type prior alone must not break a homonym tie: with
                    // two or more candidates, at least one real context term
                    // (a score of 3) is required to choose.
                    if best - second >= link_weights().multi_candidate_min_margin
                        && *best >= link_weights().multi_candidate_min_score
                    {
                        (LinkStatus::Linked, Some(*entry))
                    } else {
                        (LinkStatus::Ambiguous, None)
                    }
                }
            };

            let linked = status == LinkStatus::Linked;
            // The manifest may only publish a canonical surface the script
            // actually states. A surname promoted to the full name it was
            // aliased to ("Augias" -> "Leggere Augias") can otherwise leave an
            // identity whose canonical form never appears as its own mention;
            // the Go projection fails closed on that. Fall back to the
            // documented first-seen surface instead.
            let canonical_surface = if identity
                .mentions
                .iter()
                .any(|mention| mention.surface == identity.canonical_surface)
            {
                identity.canonical_surface.clone()
            } else {
                identity
                    .mentions
                    .first()
                    .map(|mention| mention.surface.clone())
                    .unwrap_or_else(|| identity.canonical_surface.clone())
            };
            let entity_type = if linked {
                entry
                    .map(|entry| entry.entity_type.clone())
                    .unwrap_or_else(|| identity.kind.clone())
            } else {
                identity.kind.clone()
            };
            let decision = editorial_decision(&canonical_surface, &entity_type);
            let role = role_of(&decision.kind);
            let canonical_name = if linked {
                entry
                    .map(|entry| entry.canonical_name.clone())
                    .unwrap_or_else(|| canonical_surface.clone())
            } else {
                canonical_surface.clone()
            };
            let search_intents =
                build_intents(language, identity, status, &canonical_name, identities);

            LinkedEntity {
                local_id: identity.id.clone(),
                surface_key: stable_surface_key(&identity.kind, &canonical_surface),
                canonical_surface,
                canonical_id: linked
                    .then(|| entry.map(|entry| entry.id.clone()))
                    .flatten(),
                link_status: status,
                canonical_name,
                entity_type,
                role,
                disambiguation_gloss: linked
                    .then(|| entry.map(|entry| entry.gloss.clone()))
                    .flatten(),
                negative_constraints: if linked {
                    entry
                        .map(|entry| entry.negative_constraints.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
                mentions: identity.mentions.clone(),
                search_intents,
                link_candidates,
                link_margin,
                link_rule: Some("entity_linker.v3".to_string()),
                editorial_surface: decision.surface,
                editorial_type: decision.kind,
                prompt_echo: decision.prompt_echo,
                visual_admission: decision.visual_admission,
            }
        })
        .collect()
}

pub(super) struct DiscourseSense {
    pub(super) candidate_id: String,
    pub(super) terms: Vec<String>,
    pub(super) spans: Vec<crate::identity::LinkEvidenceSpan>,
}

pub(super) fn resolved_thread_sense(
    language: &str,
    identity: &CanonicalIdentity,
    scenes: &[(&str, &str)],
    threads: &[crate::narrative::EntityThread],
    index: &EntityIndex,
) -> Option<DiscourseSense> {
    let thread = threads.iter().find(|thread| {
        thread.entity_id == identity.id
            && thread.entity_type.eq_ignore_ascii_case(&identity.kind)
            && thread.scene_ids.len() > 1
    })?;
    let candidates = index.candidates(&identity.canonical_surface);
    let mut resolved = BTreeSet::new();
    let mut evidence = BTreeMap::<String, Vec<crate::identity::LinkEvidenceSpan>>::new();
    for mention in identity
        .mentions
        .iter()
        .filter(|mention| thread.scene_ids.contains(&mention.scene_id))
    {
        let Some((_, text)) = scenes
            .iter()
            .find(|(scene_id, _)| *scene_id == mention.scene_id)
        else {
            continue;
        };
        let context = sentence_window(text, mention.start_byte, mention.end_byte);
        let mut local: Vec<(&EntityIndexEntry, i32)> = index
            .candidates(&mention.surface)
            .into_iter()
            .map(|entry| {
                (
                    entry,
                    context_score(language, entry, &identity.kind, context),
                )
            })
            .collect();
        local.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.id.cmp(&right.0.id))
        });
        let Some((winner, best)) = local.first().copied() else {
            continue;
        };
        let margin = local
            .get(1)
            .map(|second| best.saturating_sub(second.1))
            .unwrap_or(best);
        if best < link_weights().multi_candidate_min_score
            || margin < link_weights().multi_candidate_min_margin
            || !candidates.iter().any(|candidate| candidate.id == winner.id)
        {
            continue;
        }
        resolved.insert(winner.id.clone());
        let terms = winner
            .context_terms
            .iter()
            .filter(|term| contains_term(context, term))
            .cloned()
            .collect::<Vec<_>>();
        for term in terms {
            evidence
                .entry(winner.id.clone())
                .or_default()
                .extend(mention_evidence_spans(&term, mention, scenes));
        }
    }
    if resolved.len() != 1 {
        return None;
    }
    let candidate_id = resolved.into_iter().next()?;
    let mut spans = evidence.remove(&candidate_id).unwrap_or_default();
    spans.sort();
    spans.dedup();
    let terms = spans
        .iter()
        .filter_map(|span| {
            scenes
                .iter()
                .find(|(scene_id, _)| *scene_id == span.scene_id)
                .and_then(|(_, text)| text.get(span.start_byte..span.end_byte))
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    (!spans.is_empty()).then_some(DiscourseSense {
        candidate_id,
        terms,
        spans,
    })
}

pub(super) fn mention_evidence_spans(
    term: &str,
    mention: &crate::identity::Mention,
    scenes: &[(&str, &str)],
) -> Vec<crate::identity::LinkEvidenceSpan> {
    let Some((_, text)) = scenes
        .iter()
        .find(|(scene_id, _)| *scene_id == mention.scene_id)
    else {
        return Vec::new();
    };
    let (window_start, window_end) =
        sentence_window_bounds(text, mention.start_byte, mention.end_byte);
    let Some(window) = text.get(window_start..window_end) else {
        return Vec::new();
    };
    term_ranges(window, term)
        .into_iter()
        .map(|(start, end)| crate::identity::LinkEvidenceSpan {
            scene_id: mention.scene_id.clone(),
            start_byte: window_start + start,
            end_byte: window_start + end,
        })
        .collect()
}
