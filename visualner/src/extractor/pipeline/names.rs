use super::*;

/// Span set for the "organization + appositive person closing a location"
/// pattern, computed with indexes instead of the former location × organization
/// × person triple scan (O(n³) on dense scenes).
pub(super) fn appositional_location_spans<'a>(
    items: impl Iterator<Item = (&'a str, usize, usize)>,
) -> BTreeSet<(usize, usize)> {
    let mut organizations_by_start: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut persons_by_end: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut locations = Vec::new();
    for (kind, start, end) in items {
        match kind {
            "LOCATION" => locations.push((start, end)),
            "ORG" | "ORGANIZATION" => organizations_by_start.entry(start).or_default().push(end),
            "PERSON" => persons_by_end.entry(end).or_default().push(start),
            _ => {}
        }
    }
    for starts in persons_by_end.values_mut() {
        starts.sort_unstable();
    }
    let mut spans = BTreeSet::new();
    for (start, end) in locations {
        let Some(organization_ends) = organizations_by_start.get(&start) else {
            continue;
        };
        let Some(person_starts) = persons_by_end.get(&end) else {
            continue;
        };
        // The condition is existential, so the latest person start and any
        // organization ending before the location are enough.
        let latest_person_start = person_starts.last().copied().unwrap_or(0);
        if organization_ends.iter().any(|organization_end| {
            *organization_end < end && latest_person_start >= *organization_end
        }) {
            spans.insert((start, end));
        }
    }
    spans
}

pub(super) fn appositional_fragment_spans(candidates: &[Candidate]) -> BTreeSet<(usize, usize)> {
    appositional_location_spans(candidates.iter().map(|candidate| {
        (
            candidate.entity_type.as_deref().unwrap_or(""),
            candidate.start,
            candidate.end,
        )
    }))
}

pub(super) fn appositional_entity_spans(entities: &[VisualEntity]) -> BTreeSet<(usize, usize)> {
    appositional_location_spans(
        entities
            .iter()
            .map(|entity| (entity.r#type.as_str(), entity.start, entity.end)),
    )
}

/// A curated outlet ("ANSA", "la Repubblica") can be emitted twice for the
/// same span: the gazetteer name scan has no proper-noun evidence and types it
/// as a generic ORG, while the single-token scan has that evidence and emits
/// MEDIA_OUTLET. Both survived because identity keys include the type, which
/// produced two identities for one mention and a manifest whose order depended
/// on hash iteration. The outlet type is the specific one: keep it.
pub(super) fn collapse_media_outlet_duplicates(candidates: &mut Vec<Candidate>) {
    let outlets: BTreeSet<(usize, usize)> = candidates
        .iter()
        .filter(|candidate| candidate.entity_type.as_deref() == Some("MEDIA_OUTLET"))
        .map(|candidate| (candidate.start, candidate.end))
        .collect();
    if outlets.is_empty() {
        return;
    }
    // The specific type wins on an exact span: the tokenizer also yields a
    // slightly different surface for the same bytes, or an ORG from the
    // gazetteer, and keeping both would count one source twice.
    candidates.retain(|candidate| {
        candidate.entity_type.as_deref() == Some("MEDIA_OUTLET")
            || !outlets.contains(&(candidate.start, candidate.end))
    });
}

pub(super) fn collapse_media_outlet_entities(entities: Vec<VisualEntity>) -> Vec<VisualEntity> {
    let outlets: BTreeSet<(usize, usize)> = entities
        .iter()
        .filter(|entity| entity.r#type == "MEDIA_OUTLET")
        .map(|entity| (entity.start, entity.end))
        .collect();
    if outlets.is_empty() {
        return entities;
    }
    entities
        .into_iter()
        .filter(|entity| {
            entity.r#type == "MEDIA_OUTLET" || !outlets.contains(&(entity.start, entity.end))
        })
        .collect()
}

pub(super) fn is_ner_entity_type(kind: &str) -> bool {
    matches!(
        kind,
        "PERSON"
            | "ORG"
            | "ORGANIZATION"
            | "LOCATION"
            | "WORK"
            | "AWARD"
            | "EVENT"
            | "FACILITY"
            | "PRODUCT"
            | "BRAND"
    )
}

/// A civic or institutional head followed by an appositional personal name is
/// two entities, even when capitalization made the tokenizer emit one run.
/// Both sides still need independent type evidence from the shared lexicon.
pub(super) fn split_organization_person_appositions(
    candidates: &mut Vec<Candidate>,
    tokens: &[Token],
    source: &str,
    language: &str,
) {
    let mut remove = std::collections::HashSet::new();
    let mut additions = Vec::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        if candidate.text.split_whitespace().count() < 2 {
            continue;
        }
        let span_tokens: Vec<&Token> = tokens
            .iter()
            .filter(|token| token.start >= candidate.start && token.end <= candidate.end)
            .collect();
        for split in 1..span_tokens.len() {
            let prefix_end = span_tokens[split - 1].end;
            let suffix_start = span_tokens[split].start;
            let Some(prefix) = source.get(candidate.start..prefix_end) else {
                continue;
            };
            let Some(suffix) = source.get(suffix_start..candidate.end) else {
                continue;
            };
            let has_internal_name_particle =
                span_tokens[split..span_tokens.len() - 1]
                    .iter()
                    .any(|token| {
                        let base_language = language.split(['-', '_']).next().unwrap_or(language);
                        semantic_core::lexicon::lexicon()
                            .gazetteer
                            .name_particles
                            .get(base_language)
                            .is_some_and(|particles| {
                                particles
                                    .iter()
                                    .any(|particle| particle.eq_ignore_ascii_case(&token.text))
                            })
                    });
            if has_internal_name_particle {
                continue;
            }
            if values::classify_value_type(prefix, true, true, true, language) == "ORG"
                && values::classify_value_type(suffix, true, true, true, language) == "PERSON"
            {
                additions.push(Candidate {
                    text: prefix.to_string(),
                    start: candidate.start,
                    end: prefix_end,
                    entity_type: Some("ORG".to_string()),
                });
                additions.push(Candidate {
                    text: suffix.to_string(),
                    start: suffix_start,
                    end: candidate.end,
                    entity_type: Some("PERSON".to_string()),
                });
                remove.insert(candidate_index);
                break;
            }
        }
    }
    let mut index = 0;
    candidates.retain(|_| {
        let keep = !remove.contains(&index);
        index += 1;
        keep
    });
    candidates.extend(additions);
}

pub(super) fn extraction_identity_key(entity: &VisualEntity, context: &ScriptContext) -> String {
    if entity.r#type == "PERSON" && entity.text.split_whitespace().count() == 1 {
        if let Some(canonical) = context.person_surnames.get(&entity.text.to_lowercase()) {
            return format!("PERSON:{}", canonical.to_lowercase());
        }
    }
    entity_identity_key(entity)
}

pub(super) fn is_canonical_person_surface(entity: &VisualEntity, context: &ScriptContext) -> bool {
    entity.r#type == "PERSON"
        && !context
            .person_surnames
            .contains_key(&entity.text.to_lowercase())
}

/// Resolve an ambiguous proper-noun complement as an organization when a
/// language lexicon supplies a work/affiliation verb and UDPipe links the
/// name to that verb as an object or oblique argument.
pub(super) fn mark_contextual_organizations(
    candidates: &mut [Candidate],
    tokens: &[Token],
    language: &str,
) {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let lexicon = &semantic_core::lexicon::lexicon().gazetteer;
    let Some(verbs) = lexicon.organization_context_verbs.get(base) else {
        return;
    };
    for candidate in candidates {
        if candidate.entity_type.as_deref() == Some(semantic_core::entity_type_terms::MEDIA_OUTLET)
        {
            continue;
        }
        let proper_tokens: Vec<&Token> = tokens
            .iter()
            .filter(|token| {
                token.start >= candidate.start
                    && token.end <= candidate.end
                    && token.upos.as_deref() == Some("PROPN")
            })
            .collect();
        let supported = proper_tokens.iter().any(|name| {
            let Some(head) = name.head else {
                return false;
            };
            matches!(
                name.dependency.as_deref(),
                Some("obj" | "iobj" | "obl" | "nmod")
            ) && tokens.iter().any(|verb| {
                verb.id == Some(head)
                    && verb.sentence_id == name.sentence_id
                    && verb.lemma.as_deref().is_some_and(|lemma| {
                        verbs.iter().any(|known| known.eq_ignore_ascii_case(lemma))
                    })
            })
        });
        if supported {
            candidate.entity_type = Some("ORG".to_string());
        }
    }
}

/// Generic visual concepts are useful phrase inventory only when they form a
/// complete noun phrase. A bare common noun is usually too broad to identify
/// an image subject; model-backed nominal heads (for example "tifosi") are
/// useful search terms, while named entities, typed values and
/// lexicon-backed objects are handled by their own type and bypass this gate.
/// The POS check also removes adverb-led fragments such as "ancora oggi parte"
/// without maintaining a language-specific list of filler expressions.
pub(super) fn structurally_specific_visual_concept(
    entity: &VisualEntity,
    tokens: &[Token],
) -> bool {
    if entity.r#type != "VISUAL_CONCEPT" {
        return true;
    }
    let span: Vec<&Token> = tokens
        .iter()
        .filter(|token| token.start >= entity.start && token.end <= entity.end)
        .collect();
    if span.iter().all(|token| token.upos.is_none()) {
        return entity.text.split_whitespace().count() >= 2;
    }
    let has_nominal_head = span
        .iter()
        .any(|token| matches!(token.upos.as_deref(), Some("NOUN" | "PROPN")));
    let starts_as_noun_phrase = span
        .first()
        .is_some_and(|token| matches!(token.upos.as_deref(), Some("NOUN" | "PROPN" | "ADJ")));
    let ends_as_phrase = span.last().is_some_and(|token| {
        !matches!(
            token.upos.as_deref(),
            Some("ADV" | "ADP" | "AUX" | "CCONJ" | "DET" | "INTJ" | "PRON" | "SCONJ" | "VERB")
        )
    });
    has_nominal_head && starts_as_noun_phrase && ends_as_phrase
}

/// Collects the tokens of every multi-token capitalized run in the script.
/// Tokenization uses the same ICU rules as extraction, so no per-language
/// list is involved; a caseless script contributes nothing because
/// capitalization is not name evidence there.
pub fn script_person_tokens(source_texts: &[&str], language: &str) -> ScriptContext {
    let mut context = ScriptContext::default();
    let mut surnames: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if !semantic_core::lexicon::lexicon()
        .language_profiles
        .is_cased(language)
    {
        return context;
    }
    for text in source_texts {
        let Ok(tokens) = tokenize_untyped(text, language) else {
            continue;
        };
        let mut run: Vec<&Token> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let name_like = token.text.chars().next().is_some_and(char::is_uppercase);
            let particle = !run.is_empty()
                && is_name_particle(&token.text, language)
                && tokens.get(index + 1).is_some_and(|next| {
                    starts_uppercase(&next.text)
                        && !is_stop_token(next, language)
                        && !text.as_bytes()[token.end..next.start]
                            .iter()
                            .any(|byte| is_phrase_breaking_byte(*byte))
                });
            if name_like || particle {
                // A sentence break ends the name: "Garlasco. La mattina" is
                // a place and a new sentence, never the place "Garlasco La".
                if let Some(previous) = run.last() {
                    if text.as_bytes()[previous.end..token.start]
                        .iter()
                        .any(|byte| is_phrase_breaking_byte(*byte))
                    {
                        flush_name_run(&mut run, &mut context, &mut surnames, language);
                        if particle {
                            // A particle cannot open a name run.
                            continue;
                        }
                    }
                }
                run.push(token);
                continue;
            }
            flush_name_run(&mut run, &mut context, &mut surnames, language);
        }
        flush_name_run(&mut run, &mut context, &mut surnames, language);
    }
    // Only a surname that resolves to exactly one full name is an alias; two
    // people sharing it stay two people.
    for (surname, names) in surnames {
        if names.len() == 1 {
            if let Some(name) = names.into_iter().next() {
                context.person_surnames.insert(surname, name);
            }
        }
    }
    context
}
pub(super) fn flush_name_run(
    run: &mut Vec<&Token>,
    context: &mut ScriptContext,
    surnames: &mut BTreeMap<String, BTreeSet<String>>,
    language: &str,
) {
    // Two *significant* tokens make a name. "Garlasco La" is a place followed
    // by an article, so it must not contribute "garlasco" as a person token.
    let significant = run
        .iter()
        .filter(|token| !is_stop_token(token, language))
        .count();
    let usable = significant >= 2 && !run.iter().any(|token| is_heading_surface(&token.text));
    if usable {
        // A leading known place is a city, not a person token: the run
        // "Пермь Мария Соколова" must not make "пермь" look like a name
        // fragment that can suppress the LOCATION candidate later. The same
        // holds for a curated institution head: "Staatsanwaltschaft Berlin"
        // is a court, not a person name, so seeding its tokens would make
        // "berlin" look like a name fragment and drop the LOCATION candidate
        // the nested-place mechanism just produced.
        let skip_leading_place = run.len() >= 3
            && is_known_place(&run[0].text, language)
            && !is_name_particle(&run[1].text, language);
        let significant_surface = run
            .iter()
            .filter(|token| !is_stop_token(token, language))
            .map(|token| token.text.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        // A run that names an institution or a facility can end in the place it
        // serves, and that place is a LOCATION of its own: "Staatsanwaltschaft
        // Berlin", "Landgericht München", "Pengadilan Surabaya", "Università
        // di Torino", "Stadio Aurora di Torino". A person run does not: in
        // "Elena Messina" the place-looking token is a surname. The head is the
        // first *substantive* token — the leading function word of "Lo Stadio
        // Aurora di Torino" is not the head, and reading it as one would hide
        // the facility cue and seed the city as a name fragment.
        let head_surface = run
            .iter()
            .find(|token| !is_stop_token(token, language))
            .map(|token| token.text.as_str())
            .unwrap_or("");
        let institution_run = values::institution_cue_kind(&significant_surface, language)
            .is_some()
            || opens_organization_name(head_surface, language);
        let facility_run = values::opens_facility_name(head_surface);
        // Alias evidence is seeded only after the run survives typed-name
        // checks. Otherwise "Trump Tower" contributes a competing pseudo-
        // surname `Trump`, making a later `Trump` ambiguous instead of an
        // alias of Donald Trump.
        if values::curated_typed_entity(&significant_surface).is_some()
            || is_known_place(&significant_surface, language)
            || institution_run
            || facility_run
        {
            run.clear();
            return;
        }
        // A facility noun also opens ordinary names: "Stadio Aurora" is a
        // facility whose second token is part of its name, not its city, so a
        // facility keeps its trailing place only when the language's genitive
        // marker binds it ("di Torino"). An institution needs no marker: a
        // court or prosecutor's office is *of* the city it names.
        let genitive_markers = &semantic_core::lexicon::lexicon()
            .facility
            .bare_location_prepositions;
        let last_significant = run
            .iter()
            .rposition(|token| !is_stop_token(token, language));
        for (position, token) in run.iter().enumerate().skip(usize::from(skip_leading_place)) {
            if is_stop_token(token, language) {
                continue;
            }
            let follows_genitive = position > 0
                && genitive_markers
                    .iter()
                    .any(|marker| run[position - 1].text.to_lowercase() == *marker);
            let trailing_place = (position == last_significant.unwrap_or(usize::MAX))
                && (institution_run || (facility_run && follows_genitive))
                && is_known_place(&token.text, language);
            if trailing_place {
                continue;
            }
            context.person_name_tokens.insert(token.text.to_lowercase());
        }
        // A surname is only claimed from an unambiguous two-word name that is
        // neither a place nor a curated institution. "Chiara Poggi" yields
        // "poggi"; "Corte d'Assise" and "Hall of Fame" yield nothing.
        let significant_tokens: Vec<&&Token> = run
            .iter()
            .filter(|token| !is_stop_token(token, language))
            .collect();
        // Only a true two-token name contributes a surname alias. Counting
        // substantive tokens alone let a sentence-initial common noun plus a
        // particle (for example, "Immagini di Trump") compete with "Donald Trump".
        if run.len() == 2 && significant_tokens.len() == 2 {
            let first = significant_tokens[0].text.as_str();
            let last = significant_tokens[1].text.as_str();
            let surface = run
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<&str>>()
                .join(" ");
            if !is_known_place(first, language)
                && !is_known_place(last, language)
                && values::institution_cue_kind(&surface, language).is_none()
            {
                surnames
                    .entry(last.to_lowercase())
                    .or_default()
                    .insert(surface);
            }
        }
    }
    run.clear();
}

/// Drops two kinds of fragment, both derived from the script's own names:
/// a one-token place whose surface is a token of a person name stated
/// elsewhere, and a mention fully contained in a longer PERSON mention of the
/// same scene. The longer, better-typed mention owns the identity.
pub(super) fn drop_name_fragments(
    entities: Vec<VisualEntity>,
    context: &ScriptContext,
    source_text: &str,
) -> Vec<VisualEntity> {
    let person_spans: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| (entity.start, entity.end))
        .collect();
    let location_spans: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| entity.r#type == "LOCATION")
        .map(|entity| (entity.start, entity.end))
        .collect();
    // ORG and ORGANIZATION are one contract type with two spellings, so a
    // nested span is a fragment of the longer name in both readings
    // ("Comune" inside "Comune di Torino").
    let organization_spans: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| matches!(entity.r#type.as_str(), "ORG" | "ORGANIZATION"))
        .map(|entity| (entity.start, entity.end))
        .collect();
    let curated_structure_spans: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| matches!(entity.r#type.as_str(), "FACILITY" | "STRUCTURE"))
        .map(|entity| (entity.start, entity.end))
        .collect();
    let curated_compounds: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| {
            entity.text.split_whitespace().count() > 1
                && values::curated_typed_entity(&entity.text).is_some()
        })
        .map(|entity| (entity.start, entity.end))
        .collect();
    entities
        .into_iter()
        .filter(|entity| {
            if curated_compounds.iter().any(|(owner_start, owner_end)| {
                (*owner_start, *owner_end) != (entity.start, entity.end)
                    && entity.start >= *owner_start
                    && entity.end <= *owner_end
                    && !typed_compound_links_location(*owner_start, *owner_end, entity, source_text)
            }) {
                return false;
            }
            // A concept fully inside a personal or organizational name is a
            // name fragment, not B-roll vocabulary: the Dutch particle leaves
            // "Sanne" / "de" / "Vries" outside the title-case run that formed
            // "Sanne de Vries", and each fragment used to become a visual
            // concept of its own. The longer typed mention owns the span.
            if matches!(entity.r#type.as_str(), "VISUAL_CONCEPT" | "OBJECT") {
                let inside = person_spans.iter().chain(organization_spans.iter()).any(
                    |(owner_start, owner_end)| {
                        (*owner_start, *owner_end) != (entity.start, entity.end)
                            && entity.start >= *owner_start
                            && entity.end <= *owner_end
                    },
                );
                return !inside;
            }
            if !matches!(
                entity.r#type.as_str(),
                "LOCATION" | "PERSON" | "ORG" | "ORGANIZATION"
            ) {
                return true;
            }
            if entity.r#type == "LOCATION"
                && !entity.text.contains(char::is_whitespace)
                && context
                    .person_name_tokens
                    .contains(&entity.text.to_lowercase())
            {
                return false;
            }
            if entity.r#type == "LOCATION"
                && location_spans.iter().any(|(start, end)| {
                    (*start, *end) != (entity.start, entity.end)
                        && entity.start >= *start
                        && entity.end <= *end
                })
            {
                return false;
            }
            if matches!(entity.r#type.as_str(), "ORG" | "ORGANIZATION")
                && organization_spans.iter().any(|(start, end)| {
                    (*start, *end) != (entity.start, entity.end)
                        && entity.start >= *start
                        && entity.end <= *end
                })
            {
                return false;
            }
            if entity.r#type == "PERSON"
                && curated_structure_spans.iter().any(|(start, end)| {
                    (*start, *end) != (entity.start, entity.end)
                        && entity.start >= *start
                        && entity.end <= *end
                })
            {
                return false;
            }
            if entity.r#type == "PERSON"
                && organization_spans.iter().any(|(start, end)| {
                    (*start, *end) != (entity.start, entity.end)
                        && entity.start >= *start
                        && entity.end <= *end
                })
            {
                return false;
            }
            !person_spans.iter().any(|(start, end)| {
                (*start, *end) != (entity.start, entity.end)
                    && entity.start >= *start
                    && entity.end <= *end
            })
        })
        .collect()
}

pub(super) fn typed_compound_links_location(
    owner_start: usize,
    _owner_end: usize,
    entity: &VisualEntity,
    source_text: &str,
) -> bool {
    if entity.r#type != "LOCATION" || entity.start <= owner_start {
        return false;
    }
    let Some(prefix) = source_text.get(owner_start..entity.start) else {
        return false;
    };
    let prefix = prefix.trim_end().to_lowercase();
    semantic_core::lexicon::lexicon()
        .facility
        .bare_location_prepositions
        .iter()
        .any(|cue| prefix.ends_with(cue))
}

/// entities (NUMBER/PERCENT/MONEY). Without this, a low cap ranks every value
/// below the identities and the chart loses its numbers entirely. The share is
/// a third of the cap (at least one slot); numeric entities beyond the cap
/// replace the lowest-ranked non-numeric entities only when fewer numerics
/// made the cut. Surviving entities keep their original rank order.
pub(super) fn reserve_numeric_slots(ranked: Vec<VisualEntity>, top_n: usize) -> Vec<VisualEntity> {
    if ranked.len() <= top_n {
        return ranked;
    }
    let is_numeric = |e: &VisualEntity| {
        matches!(
            e.r#type.as_str(),
            "NUMBER" | "PERCENT" | "MONEY" | "MEASUREMENT"
        )
    };
    let reserve = (top_n / 3).max(1).min(top_n);
    let kept_numeric = ranked[..top_n].iter().filter(|e| is_numeric(e)).count();
    let spare: Vec<usize> = (top_n..ranked.len())
        .filter(|&i| is_numeric(&ranked[i]))
        .collect();
    let needed = reserve.saturating_sub(kept_numeric).min(spare.len());
    let mut keep = vec![true; top_n];
    let mut evicted = 0;
    for pos in (0..top_n).rev() {
        if evicted == needed {
            break;
        }
        if !is_numeric(&ranked[pos]) {
            keep[pos] = false;
            evicted += 1;
        }
    }
    let mut selected: Vec<usize> = (0..top_n)
        .filter(|&pos| keep[pos])
        .chain(spare.into_iter().take(needed))
        .collect();
    selected.sort_unstable();
    let mut slots: Vec<Option<VisualEntity>> = ranked.into_iter().map(Some).collect();
    selected
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}
