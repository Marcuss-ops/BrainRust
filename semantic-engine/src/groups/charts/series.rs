use super::*;

/// Uses the semantic quantity attached to this exact numeric span when one is
/// available. The legacy chart parser remains a compatibility fallback for
/// chart points that have no recognized typed unit.
///
/// Owner: `semantic-engine` chart proposal builder.
/// Removal: 2027-03-31, once every chart point carries a typed quantity.
/// Ticket: `docs/tickets/code-hygiene-2026-11-24/07-LEGACY-BRANCHES-AND-ARGS.md`.
/// Covered by `a_point_without_a_typed_quantity_uses_the_legacy_parser`.
pub(super) fn chart_measure(
    sentence: &str,
    start: usize,
    end: usize,
    point: &ChartPoint,
    quantities: &[Quantity],
) -> crate::chart_rules::ChartUnit {
    if let Some(quantity) = quantities
        .iter()
        .find(|quantity| quantity.start_byte == point.start && quantity.end_byte == point.end)
    {
        if let (Some(raw_value), Some(_)) = (quantity.raw_value, quantity.value) {
            let scaled_value = raw_value * quantity.scale.unwrap_or(1.0);
            let kind = match quantity.dimension.as_str() {
                "percent" => "PERCENT",
                "currency" => "MONEY",
                _ => point.kind.as_str(),
            };
            let display_measure =
                crate::chart_rules::chart_unit(sentence, start, end, scaled_value, kind);
            let signature = match quantity.dimension.as_str() {
                "length" => "length:m".to_string(),
                "currency" => {
                    let period = display_measure
                        .signature
                        .rsplit_once('/')
                        .map_or("unit", |(_, period)| period);
                    format!("currency:{}/{period}", quantity.unit)
                }
                _ => format!("{}:{}", quantity.dimension, quantity.unit),
            };
            return crate::chart_rules::ChartUnit {
                kind: kind.to_string(),
                signature,
                value: display_measure.value,
            };
        }
    }
    crate::chart_rules::chart_unit(sentence, start, end, point.value, &point.kind)
}

/// Joins adjacent source-grounded measures when the script repeats a metric
/// across sentences (for example, annual revenue or output). Consecutive
/// points with the same metric and unit form ONE ordered series of 2..=6
/// items; the emitted evidence is one exact source slice spanning all included
/// sentences. A grounded `Quantity` predicate and unit take precedence over the
/// lexical stem fallback.
/// Inputs for the cross-sentence chart-series pass.
/// The sentence-level bindings a series point needs. Grouped so the chain
/// walker keeps one context argument instead of six unrelated slices.
struct SeriesBindings<'a> {
    bullets: &'a [Bullet],
    quantities: &'a [Quantity],
    entities: &'a [VisualEntity],
    tokens: &'a [AnalyzedToken],
    identities: &'a IdentityLookup,
    language: &'a str,
}

pub(super) struct SeriesContext<'a> {
    pub(super) source: &'a str,
    pub(super) entities: &'a [VisualEntity],
    pub(super) bullets: &'a [Bullet],
    pub(super) chart_points: &'a [ChartPoint],
    pub(super) quantities: &'a [Quantity],
    pub(super) tokens: &'a [AnalyzedToken],
    pub(super) identities: &'a IdentityLookup,
    pub(super) language: &'a str,
}

/// Inputs for one candidate assembled from an already-selected chain.
struct SeriesCandidateContext<'a, 'b> {
    source: &'a str,
    entities: &'a [VisualEntity],
    bullets: &'a [Bullet],
    ordered: &'a [&'b ChartPoint],
    chain: &'a [usize],
    quantities: &'a [Quantity],
    identities: &'a IdentityLookup,
    language: &'a str,
}

pub(super) fn cross_sentence_series(input: SeriesContext<'_>, out: &mut Vec<VisualCandidate>) {
    let SeriesContext {
        source,
        entities,
        bullets,
        chart_points,
        quantities,
        tokens,
        identities,
        language,
    } = input;
    let bindings = SeriesBindings {
        bullets,
        quantities,
        entities,
        tokens,
        identities,
        language,
    };
    let mut ordered: Vec<&ChartPoint> = chart_points.iter().collect();
    ordered.sort_by_key(|point| (point.start, point.end));
    let mut used = vec![false; ordered.len()];
    for start_index in 0..ordered.len() {
        if used[start_index] {
            continue;
        }
        let mut chain = vec![start_index];
        let mut cursor = start_index;
        while chain.len() < 6 {
            let Some(next) = next_series_point(&ordered, &used, cursor, &bindings) else {
                break;
            };
            chain.push(next);
            cursor = next;
        }
        if chain.len() < 2 {
            continue;
        }
        let spans: Vec<(usize, usize)> = chain
            .iter()
            .map(|index| {
                let point = ordered[*index];
                (point.start, point.end)
            })
            .collect();
        if crate::chart_rules::mixed_forecast_claims(source, &spans) {
            continue;
        }
        let Some(candidate) = build_series_candidate(SeriesCandidateContext {
            source,
            entities,
            bullets,
            ordered: &ordered,
            chain: &chain,
            quantities,
            identities,
            language,
        }) else {
            continue;
        };
        // A single sentence-local chart already covering these exact values
        // wins; do not emit a duplicate scenario-spanning copy.
        if out.iter().any(|existing| {
            existing.kind == "chart"
                && chain.iter().all(|index| {
                    let point = ordered[*index];
                    existing
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == point.start && item.end_byte == point.end)
                })
        }) {
            continue;
        }
        for index in &chain {
            used[*index] = true;
        }
        out.push(candidate);
    }
}

/// Finds the next ordered point that extends the current series. Gap, metric,
/// unit and time-anchor checks are identical to the historical pair rule; the
/// difference is that the chain is kept together instead of fragmenting into
/// overlapping pairs.
fn next_series_point(
    ordered: &[&ChartPoint],
    used: &[bool],
    from: usize,
    bindings: &SeriesBindings<'_>,
) -> Option<usize> {
    let SeriesBindings {
        bullets,
        quantities,
        entities,
        tokens,
        identities,
        language,
    } = *bindings;
    let left = ordered[from];
    let left_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == left.sentence_index)?;
    for index in from + 1..ordered.len() {
        if used[index] {
            continue;
        }
        let right = ordered[index];
        if right.sentence_index <= left.sentence_index {
            continue;
        }
        if right.sentence_index.saturating_sub(left.sentence_index) > 3 {
            break;
        }
        let Some(right_bullet) = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == right.sentence_index)
        else {
            continue;
        };
        if !crate::chart_rules::chart_point_is_eligible(
            &left_bullet.text,
            left.start.saturating_sub(left_bullet.start),
            left.end.saturating_sub(left_bullet.start),
        ) || !crate::chart_rules::chart_point_is_eligible(
            &right_bullet.text,
            right.start.saturating_sub(right_bullet.start),
            right.end.saturating_sub(right_bullet.start),
        ) {
            continue;
        }
        if !series_time_anchor(&left_bullet.text, &right_bullet.text) {
            continue;
        }
        // A repeated metric under different years is only one series when the
        // values belong to the same subject. "Rick Ross ... Eminem ..." must
        // never become a Rick Ross trend.
        if !same_series_subject(
            left_bullet,
            right_bullet,
            entities,
            tokens,
            identities,
            language,
        ) {
            continue;
        }
        let (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) = (
            left.start.checked_sub(left_bullet.start),
            left.end.checked_sub(left_bullet.start),
            right.start.checked_sub(right_bullet.start),
            right.end.checked_sub(right_bullet.start),
        ) else {
            continue;
        };
        let left_measure = chart_measure(&left_bullet.text, left_start, left_end, left, quantities);
        let right_measure = chart_measure(
            &right_bullet.text,
            right_start,
            right_end,
            right,
            quantities,
        );
        if !compatible_measures(&left_measure, &right_measure) {
            continue;
        }
        let metric = series_metric_match(
            &left_bullet.text,
            &right_bullet.text,
            quantities,
            left,
            right,
            language,
        );
        if metric.is_none() {
            // A repeated four-digit year anchor is itself the shared
            // dimension: unitless values under consecutive years form a time
            // series even when the sentence names no metric noun.
            let unitless = |measure: &crate::chart_rules::ChartUnit| {
                measure.kind == "NUMBER" && measure.signature == measure.kind
            };
            if !(unitless(&left_measure) && unitless(&right_measure)) {
                continue;
            }
        }
        return Some(index);
    }
    None
}

pub(super) fn compatible_measures(
    left: &crate::chart_rules::ChartUnit,
    right: &crate::chart_rules::ChartUnit,
) -> bool {
    if left.signature == right.signature && left.kind == right.kind {
        return true;
    }
    let fallback = |bare: &crate::chart_rules::ChartUnit,
                    explicit: &crate::chart_rules::ChartUnit| {
        bare.signature == bare.kind && explicit.signature != explicit.kind
    };
    (fallback(left, right) || fallback(right, left))
        && (left.kind == right.kind
            || left.signature == right.signature
            || right.signature == right.kind
            || left.signature == left.kind)
}

/// Grounded entity families that can own a chart series. Generic visual
/// concepts and measured values are not subjects.
fn is_chart_subject_type(kind: &str) -> bool {
    matches!(
        kind,
        "PERSON" | "ORG" | "FACILITY" | "LOCATION" | "GPE" | "PRODUCT" | "EVENT" | "NORP" | "LAW"
    )
}

/// The subject a sentence attributes its value to, when unambiguous.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SeriesSubject {
    One(String),
    /// No person is named: a pronoun or an elided subject continues the
    /// previous series without competing with it.
    Unnamed,
    /// More than one person: the value cannot be attributed by position.
    Ambiguous,
}

pub(super) fn series_subject(
    bullet: &Bullet,
    entities: &[VisualEntity],
    tokens: &[AnalyzedToken],
    identities: &IdentityLookup,
    language: &str,
) -> SeriesSubject {
    let mut subjects = Vec::new();
    for (index, entity) in entities.iter().enumerate() {
        if !is_chart_subject_type(&entity.r#type)
            || entity.start < bullet.start
            || entity.end > bullet.end
        {
            continue;
        }
        // The shared registry collapses a bare surname and its full name to
        // the same id; compare ids, not surfaces.
        let id = identities.id_of(index);
        if !subjects.contains(&id) {
            subjects.push(id);
        }
    }
    let _ = language;
    match subjects.len() {
        0 => syntactic_subject(bullet, tokens),
        1 => {
            let id = subjects.remove(0);
            // A coordinated second name ("Rick Ross ed Eminem") may fall
            // outside the entity type (VISUAL_CONCEPT), but the value still
            // belongs to two subjects; the syntax alone sees it.
            match syntactic_subject(bullet, tokens) {
                SeriesSubject::Ambiguous => SeriesSubject::Ambiguous,
                _ => SeriesSubject::One(id),
            }
        }
        _ => SeriesSubject::Ambiguous,
    }
}

/// Fallback for a proper name the named-entity pass missed ("Eminem"): the UD
/// `nsubj` of the sentence is still a subject, and two different surfaces must
/// not become one series. Pronouns and elided subjects continue a chain.
fn syntactic_subject(bullet: &Bullet, tokens: &[AnalyzedToken]) -> SeriesSubject {
    let local: Vec<&AnalyzedToken> = tokens
        .iter()
        .filter(|token| token.start >= bullet.start && token.end <= bullet.end)
        .collect();
    let subjects: Vec<&&AnalyzedToken> = local
        .iter()
        .filter(|token| {
            matches!(
                token.dependency.as_deref(),
                Some("nsubj") | Some("nsubj:pass")
            )
        })
        .collect();
    if subjects.is_empty() {
        // UDPipe's Italian model tags a sentence-initial unknown name as NOUN
        // (`Eminem`), not PROPN, and does not always attach it as `nsubj`. A
        // capitalized lemma still marks a named subject, so a change of name
        // breaks the chain instead of inheriting the previous subject.
        return named_subject(&local);
    }
    if subjects.len() > 1 {
        return SeriesSubject::Ambiguous;
    }
    let head = *subjects[0];
    // A clitic pronoun, a bare number parsed as the subject ("ne ha prodotti
    // 120.000") or any other non-nominal head is not a competing named
    // subject: the sentence continues the chain.
    if !matches!(head.upos.as_deref(), Some("NOUN") | Some("PROPN")) {
        return SeriesSubject::Unnamed;
    }
    // "Rick Ross ed Eminem": the coordinated conjunct is a second subject.
    if local.iter().any(|token| {
        token.dependency.as_deref() == Some("conj") && token.head.is_some() && token.head == head.id
    }) {
        return SeriesSubject::Ambiguous;
    }
    // "i ricavi di Alfa": the grammatical subject is the metric noun, but the
    // series belongs to the named entity in its subtree. A common noun with no
    // named dependent keeps its own surface, so two different metrics do not
    // merge by accident.
    let subtree = subtree_tokens(&local, head.id);
    if head.upos.as_deref() == Some("NOUN") {
        if let Some(named) = named_surface(&subtree) {
            return named;
        }
    }
    let mut part_ids: HashSet<Option<i32>> = HashSet::new();
    part_ids.insert(head.id);
    loop {
        let before = part_ids.len();
        for token in &local {
            if let (Some(id), Some(token_head)) = (token.id, token.head) {
                if part_ids.contains(&Some(token_head))
                    && matches!(
                        token.dependency.as_deref(),
                        Some("flat") | Some("compound") | Some("fixed") | Some("appos")
                    )
                {
                    part_ids.insert(Some(id));
                }
            }
        }
        if part_ids.len() == before {
            break;
        }
    }
    let mut surface: Vec<&AnalyzedToken> = local
        .iter()
        .filter(|token| part_ids.contains(&token.id))
        .copied()
        .collect();
    surface.sort_by_key(|token| token.start);
    let surface = surface
        .iter()
        .map(|token| token.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    if surface.is_empty() {
        SeriesSubject::Unnamed
    } else {
        SeriesSubject::One(format!("surface:{surface}"))
    }
}

/// Two consecutive values join one series only when they name the same subject
/// or one sentence leaves the subject implicit. A different named subject and
/// an ambiguous sentence both break the chain.
pub(super) fn same_series_subject(
    left: &Bullet,
    right: &Bullet,
    entities: &[VisualEntity],
    tokens: &[AnalyzedToken],
    identities: &IdentityLookup,
    language: &str,
) -> bool {
    let left = series_subject(left, entities, tokens, identities, language);
    let right = series_subject(right, entities, tokens, identities, language);
    match (left, right) {
        (SeriesSubject::One(a), SeriesSubject::One(b)) => a == b,
        (SeriesSubject::Ambiguous, _) | (_, SeriesSubject::Ambiguous) => false,
        _ => true,
    }
}

/// Groups the named tokens of a sentence by their own head (name parts are
/// `flat`/`compound`), so `Rick Ross` is one subject and `Rick Ross ed Eminem`
/// is two. Zero groups is an elided subject; two or more are ambiguous.
fn named_subject(local: &[&AnalyzedToken]) -> SeriesSubject {
    named_surface(local).unwrap_or(SeriesSubject::Unnamed)
}

fn named_surface(local: &[&AnalyzedToken]) -> Option<SeriesSubject> {
    let is_named = |token: &&AnalyzedToken| {
        token.upos.as_deref() == Some("PROPN")
            || (token.upos.as_deref() == Some("NOUN")
                && token
                    .lemma
                    .as_deref()
                    .and_then(|lemma| lemma.chars().next())
                    .is_some_and(char::is_uppercase))
    };
    let named: Vec<&AnalyzedToken> = local.iter().copied().filter(is_named).collect();
    if named.is_empty() {
        return None;
    }
    let mut groups: Vec<Vec<&AnalyzedToken>> = Vec::new();
    for token in &named {
        let part_of_name = matches!(
            token.dependency.as_deref(),
            Some("flat") | Some("compound") | Some("fixed") | Some("appos")
        );
        if part_of_name {
            if let Some(group) = groups.iter_mut().find(|group| {
                group
                    .iter()
                    .any(|head| head.id.is_some() && head.id == token.head)
            }) {
                group.push(token);
                continue;
            }
        }
        groups.push(vec![token]);
    }
    if groups.len() > 1 {
        return Some(SeriesSubject::Ambiguous);
    }
    let mut group = groups.remove(0);
    group.sort_by_key(|token| token.start);
    let surface = group
        .iter()
        .map(|token| token.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    Some(SeriesSubject::One(format!("surface:{surface}")))
}

fn subtree_tokens<'a>(local: &[&'a AnalyzedToken], root: Option<i32>) -> Vec<&'a AnalyzedToken> {
    let mut ids: HashSet<Option<i32>> = HashSet::new();
    ids.insert(root);
    loop {
        let before = ids.len();
        for token in local {
            if token.head.is_some() && ids.contains(&token.head) {
                ids.insert(token.id);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    local
        .iter()
        .copied()
        .filter(|token| ids.contains(&token.id))
        .collect()
}

pub(super) fn series_time_anchor(left: &str, right: &str) -> bool {
    if !has_four_digit_year(left) {
        return false;
    }
    let lower = right.to_lowercase();
    has_four_digit_year(right) || lower.contains("seguente") || lower.contains("following year")
}

/// Resolves the repeated metric for a series. Exact grounded quantities win:
/// two values with the same dimension/unit and the same explicit predicate are
/// the same metric even when the surface wording differs. When the predicate is
/// missing, the language stem table is the conservative fallback.
pub(super) fn series_metric_match(
    left_text: &str,
    right_text: &str,
    quantities: &[Quantity],
    left: &ChartPoint,
    right: &ChartPoint,
    language: &str,
) -> Option<String> {
    let exact = |point: &ChartPoint| {
        quantities
            .iter()
            .find(|quantity| quantity.start_byte == point.start && quantity.end_byte == point.end)
    };
    if let (Some(left_quantity), Some(right_quantity)) = (exact(left), exact(right)) {
        if left_quantity.dimension == right_quantity.dimension
            && left_quantity.unit == right_quantity.unit
        {
            match (&left_quantity.predicate, &right_quantity.predicate) {
                (Some(a), Some(b)) if a == b => return Some(a.clone()),
                (Some(_), Some(_)) => return None,
                _ => {}
            }
        }
    }
    let left_lower = left_text.to_lowercase();
    let right_lower = right_text.to_lowercase();
    semantic_core::chart_cues::language_cues(language)
        .series_metric_stems
        .iter()
        .find(|stem| left_lower.contains(stem.as_str()) && right_lower.contains(stem.as_str()))
        .cloned()
}

fn build_series_candidate(input: SeriesCandidateContext<'_, '_>) -> Option<VisualCandidate> {
    let SeriesCandidateContext {
        source,
        entities,
        bullets,
        ordered,
        chain,
        quantities,
        identities,
        language,
    } = input;
    let points: Vec<&ChartPoint> = chain.iter().map(|index| ordered[*index]).collect();
    let first = *points.first()?;
    let last = *points.last()?;
    let first_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == first.sentence_index)?;
    let last_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == last.sentence_index)?;
    let (evidence_start, _) = chart_claim_bounds(first_bullet, &[first]);
    let (_, evidence_end) = chart_claim_bounds(last_bullet, &[last]);
    let evidence = source.get(evidence_start..evidence_end)?;

    let mut measures: Vec<crate::chart_rules::ChartUnit> = Vec::with_capacity(points.len());
    for point in &points {
        let bullet = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == point.sentence_index)?;
        let start = point.start.checked_sub(bullet.start)?;
        let end = point.end.checked_sub(bullet.start)?;
        measures.push(chart_measure(&bullet.text, start, end, point, quantities));
    }
    // A bare number beside an explicit unit ("120 cars" then "180") inherits
    // the explicit unit, exactly like the sentence-local chart path.
    inherit_explicit_units(&mut measures);
    if measures
        .windows(2)
        .any(|pair| !compatible_measures(&pair[0], &pair[1]))
    {
        return None;
    }
    let mut items = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let bullet = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == point.sentence_index)?;
        let start = point.start.checked_sub(bullet.start)?;
        let end = point.end.checked_sub(bullet.start)?;
        let measure = &measures[index];
        let (display_start, display_end) =
            crate::chart_rules::qualified_numeric_span(&bullet.text, start, end);
        let text = bullet.text.get(display_start..display_end)?;
        // A cross-sentence annual series names each year once, so the single
        // year in the sentence is the correct category for that point.
        let label = single_year_label(&bullet.text).unwrap_or_else(|| {
            chart_subject_label(
                source,
                entities,
                point,
                bullet,
                bullet.start,
                bullet.end,
                language,
            )
        });
        let quantity =
            chart_point_quantity(quantities, point.sentence_index, point.start, point.end);
        items.push(ChartItem {
            label,
            text: text.to_string(),
            kind: measure.kind.clone(),
            value: measure.value,
            start_byte: bullet.start + display_start,
            end_byte: bullet.start + display_end,
            qualifier: quantity
                .map(|quantity| quantity.qualifier.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| crate::chart_rules::numeric_qualifier(&bullet.text, start, end)),
            period: quantity
                .and_then(|quantity| quantity.period.as_ref())
                .map(|period| period.year.to_string()),
            lower: None,
            upper: None,
            range_id: quantity.and_then(|quantity| quantity.range_id.clone()),
        });
    }
    if items.len() < 2 {
        return None;
    }
    let mut indices: Vec<usize> = points.iter().filter_map(|point| point.entity).collect();
    indices.sort_unstable();
    indices.dedup();
    let chart_meta = chart_metadata(&items, &measures, quantities, &points);
    let suggested_layout = select_layout(&items, evidence, language);
    Some(VisualCandidate {
        kind: "chart",
        sentence_index: first.sentence_index.min(last.sentence_index),
        entity_ids: ids_of(identities, &indices),
        entity_texts: texts_of(entities, &indices),
        entity_indices: indices,
        chart_items: items,
        chart_meta,
        temporal_surfaces: Vec::new(),
        suggested_layout: Some(suggested_layout),
        requires_verified_images: false,
        evidence: evidence.to_string(),
        start: evidence_start,
        end: evidence_end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_without_a_typed_quantity_uses_the_legacy_parser() {
        let point = ChartPoint {
            text: "42".to_string(),
            kind: "count".to_string(),
            value: 42.0,
            label: "42".to_string(),
            sentence_index: 0,
            start: 0,
            end: 2,
            entity: None,
        };
        let fallback = chart_measure("42", 0, 2, &point, &[]);
        let expected = crate::chart_rules::chart_unit("42", 0, 2, 42.0, "count");
        assert_eq!(fallback.kind, expected.kind);
        assert_eq!(fallback.signature, expected.signature);
        assert_eq!(fallback.value, expected.value);
    }
}
