//! Versioned linguistic knowledge, kept OUT of Rust code.
//!
//! Every curated vocabulary this workspace used to hard-code in Rust
//! (stopwords, boilerplate phrases, object vocabularies, location cues, scale
//! words, legal markers, discourse cues, process phases) lives in JSON tables
//! under `lexicon/`. Rust owns the
//! mechanism (tokenization, spans, scoring, grounding); the lexicon owns the
//! knowledge. Adding or correcting a word is a data change, never a code
//! change, so a new language or editorial domain scales without recompiling.
//!
//! Loading is deterministic: the embedded JSON is the built-in default and
//! the same input always produces the same analysis. Setting
//! `SEMANTIC_LEXICON_DIR` to a directory containing any of the table files
//! replaces those tables wholesale (never merged) so a deployment can extend
//! or trim the vocabulary without a rebuild; a malformed override is ignored
//! with a warning and the embedded default keeps running.
//!
//! The tables are versioned (`LEXICON_VERSION`) and recorded in
//! [`crate::AnalyzerVersions`], so a vocabulary change can never serve a
//! stale cached analysis.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Version of the lexicon tables. Bump whenever a table changes semantics.
pub const LEXICON_VERSION: &str = "lexicon.v4";

/// Environment variable naming a directory of overriding table files.
pub const LEXICON_DIR_ENV: &str = "SEMANTIC_LEXICON_DIR";

// ── Table shapes ────────────────────────────────────────────────────

/// Candidate-exclusion vocabularies (visualner stopword fallback).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stopwords {
    pub stop_words: Vec<String>,
    pub non_ascii_stop_words: Vec<String>,
    pub stop_phrases: Vec<String>,
    pub title_words: Vec<String>,
    /// Narration boilerplate that must never become an entity
    /// ("Imagine the", "Get ready", ...).
    pub generic_phrases: Vec<String>,
    /// Articulated prepositions that UDPipe splits into preposition +
    /// article ("del" -> "di" + "il"). The tagger alignment uses this closed
    /// morphosyntactic class to keep the contraction's tag.
    #[serde(default)]
    pub articulated_prepositions: Vec<String>,
}

/// Day/month order a numeric date uses when both readings are calendar-legal
/// (`03/04/2020`). Only the convention `date_order` records is applied; a
/// source that cannot be read either way stays unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    DayFirst,
    MonthFirst,
}

/// One language's pipeline profile.
///
/// These are the facts the *mechanism* needs to behave correctly in a
/// language: which UDPipe treebank supplies tags and lemmas, which date order
/// its writers use, how it groups and splits numbers, and whether its verbal
/// morphology is certified for predicate filtering. They live here, in data,
/// so adding a language is an entry — never a `match` in Rust.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LanguageProfile {
    /// UDPipe treebank whose model supplies POS tags and lemmas. Absent when
    /// no model is certified: the extractor then runs the model-free
    /// fallback instead of guessing.
    #[serde(default)]
    pub treebank: Option<String>,
    /// Writing system (key into [`LanguageProfiles::scripts`]). Mandatory for
    /// every non-`default` entry: the case behavior of the script decides
    /// whether an uppercase initial is evidence of a proper name.
    #[serde(default)]
    pub script: Option<String>,
    /// The orthography capitalizes common nouns (German). Capitalization then
    /// stops being evidence of a name: every noun is capitalized.
    #[serde(default)]
    pub capitalized_nouns: bool,
    #[serde(default)]
    pub date_order: Option<DateOrder>,
    #[serde(default)]
    pub thousands_separator: Option<char>,
    #[serde(default)]
    pub decimal_separator: Option<char>,
    /// Enables the verbal-predicate filter (tag `VERB`/`AUX` plus the lemma
    /// fallback). Off until a language's morphology is certified.
    #[serde(default)]
    pub verb_filter: bool,
    /// Infinitive endings that let the lemma prove an inflected form
    /// (`guarda` → `guardare`). Empty disables the lemma branch: the tag
    /// alone is never enough to overrule a noun with a nominal lemma.
    #[serde(default)]
    pub infinitive_suffixes: Vec<String>,
}

/// One writing system's case behavior. Scripts without letter case (Han,
/// Japanese, Hangul, Arabic, Devanagari, Thai) have no uppercase initial, so
/// requiring one would make every proper name in those scripts invisible.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScriptProfile {
    /// Mandatory: a script that forgets to declare this must not be guessed.
    #[serde(default)]
    pub cased: Option<bool>,
}

/// Per-language profiles, keyed by ISO-639-1 base code, plus the writing
/// systems they use. The `default` entry supplies any field a language does
/// not override.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LanguageProfiles {
    #[serde(default)]
    pub profiles: BTreeMap<String, LanguageProfile>,
    #[serde(default)]
    pub scripts: BTreeMap<String, ScriptProfile>,
}

impl LanguageProfiles {
    fn entry(&self, language: &str) -> Option<&LanguageProfile> {
        let key = language
            .trim()
            .split(['-', '_'])
            .next()?
            .to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        self.profiles.get(&key)
    }

    /// The merged view: the language's own entry, falling back to `default`.
    fn effective(&self, language: &str) -> Option<&LanguageProfile> {
        self.entry(language).or_else(|| self.entry("default"))
    }

    /// UDPipe treebank for a BCP-47 tag. Deliberately does NOT fall back to
    /// `default`: a language without a certified model must run the
    /// model-free path, never another language's model.
    pub fn treebank(&self, language: &str) -> Option<&str> {
        self.entry(language)?.treebank.as_deref()
    }

    /// Writing system of a language, when the table declares one.
    pub fn script(&self, language: &str) -> Option<&ScriptProfile> {
        let name = self.entry(language)?.script.as_deref()?;
        self.scripts.get(name)
    }

    /// True when the language's script has letter case. An unknown language
    /// keeps the historical cased behavior: only an explicit uncased script
    /// entry removes the capitalization requirement.
    pub fn is_cased(&self, language: &str) -> bool {
        self.script(language)
            .is_none_or(|script| script.cased.unwrap_or(true))
    }

    /// True when the orthography capitalizes common nouns (German).
    pub fn capitalized_nouns(&self, language: &str) -> bool {
        self.entry(language)
            .is_some_and(|profile| profile.capitalized_nouns)
    }

    /// The pivot: does an uppercase initial prove a proper name? Only in a
    /// cased script that does not capitalize ordinary nouns. In German every
    /// noun is capitalized, and in Han/Japanese/Arabic/Thai there is no case,
    /// so the capitalization test carries no information there.
    pub fn uppercase_is_name_evidence(&self, language: &str) -> bool {
        self.is_cased(language) && !self.capitalized_nouns(language)
    }

    /// Day/month order for numeric dates (day-first when unconfigured).
    pub fn date_order(&self, language: &str) -> DateOrder {
        self.effective(language)
            .and_then(|profile| profile.date_order)
            .unwrap_or(DateOrder::DayFirst)
    }

    pub fn thousands_separator(&self, language: &str) -> char {
        self.effective(language)
            .and_then(|profile| profile.thousands_separator)
            .unwrap_or('.')
    }

    pub fn decimal_separator(&self, language: &str) -> char {
        self.effective(language)
            .and_then(|profile| profile.decimal_separator)
            .unwrap_or(',')
    }

    pub fn verb_filter(&self, language: &str) -> bool {
        self.effective(language)
            .is_some_and(|profile| profile.verb_filter)
    }

    pub fn infinitive_suffixes(&self, language: &str) -> &[String] {
        self.entry(language)
            .map(|profile| profile.infinitive_suffixes.as_slice())
            .unwrap_or(&[])
    }

    /// Checks the table's own invariants. An empty result means every profile
    /// is usable; a non-empty result names the broken entry, so a data edit
    /// fails a test instead of silently degrading an analysis.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.scripts.is_empty() {
            problems.push("no `scripts` table".to_string());
        }
        for (code, script) in &self.scripts {
            if code.trim().is_empty() {
                problems.push("a script entry has an empty name".to_string());
            }
            if script.cased.is_none() {
                problems.push(format!("script {code:?}: missing mandatory `cased`"));
            }
        }
        for (code, profile) in &self.profiles {
            let is_default = code == "default";
            if !is_default && (code.len() != 2 || !code.chars().all(|c| c.is_ascii_lowercase())) {
                problems.push(format!(
                    "profile {code:?}: expected a two-letter lowercase language code"
                ));
            }
            if is_default {
                continue;
            }
            match profile.script.as_deref() {
                None => problems.push(format!("profile {code:?}: missing mandatory `script`")),
                Some(script) if !self.scripts.contains_key(script) => {
                    problems.push(format!("profile {code:?}: unknown script {script:?}"))
                }
                Some(_) => {}
            }
            if let Some(treebank) = profile.treebank.as_deref() {
                if treebank.trim().is_empty() {
                    problems.push(format!("profile {code:?}: empty `treebank`"));
                }
            }
            if profile.verb_filter && profile.infinitive_suffixes.is_empty() {
                problems.push(format!(
                    "profile {code:?}: `verb_filter` needs at least one `infinitive_suffixes` entry"
                ));
            }
            if profile
                .infinitive_suffixes
                .iter()
                .any(|suffix| suffix.trim().is_empty())
            {
                problems.push(format!(
                    "profile {code:?}: an `infinitive_suffixes` entry is empty"
                ));
            }
            if let (Some(thousands), Some(decimal)) =
                (profile.thousands_separator, profile.decimal_separator)
            {
                if thousands == decimal {
                    problems.push(format!(
                        "profile {code:?}: thousands and decimal separators are both {thousands:?}"
                    ));
                }
            }
        }
        problems
    }
}

/// Sentence-boundary abbreviations: a token ending in one of these terminates
/// a clause, not the sentence. `common` applies to every language; a language
/// adds its own forms through `by_language`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Abbreviations {
    #[serde(default)]
    pub common: Vec<String>,
    /// Company suffixes (`Inc.`) that only suppress the boundary before a
    /// lowercase continuation ("Acme Inc. and ..." stays one sentence).
    #[serde(default)]
    pub corporate: Vec<String>,
    #[serde(default)]
    pub by_language: BTreeMap<String, Vec<String>>,
}

impl Abbreviations {
    /// Every abbreviation that suppresses a sentence boundary in `language`.
    pub fn for_language(&self, language: &str) -> impl Iterator<Item = &str> {
        let key = language.split(['-', '_']).next().unwrap_or("");
        let localized = self.by_language.get(key).map(Vec::as_slice).unwrap_or(&[]);
        self.common
            .iter()
            .chain(localized.iter())
            .map(String::as_str)
    }
}

/// One era marker and the era it denotes (`bce`/`ce`).
#[derive(Debug, Clone, Deserialize)]
pub struct EraMarker {
    pub marker: String,
    pub era: String,
}

/// Orthographic knowledge used by sentence splitting and date recognition:
/// abbreviation sets and era markers. Kept in data so a new language's forms
/// never require a code change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Orthography {
    #[serde(default)]
    pub abbreviations: Abbreviations,
    #[serde(default)]
    pub era_markers: Vec<EraMarker>,
}

/// Per-language visual-object vocabulary (typed OBJECT candidates).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ObjectVocabulary {
    pub terms: BTreeMap<String, Vec<String>>,
}

/// One place from the GeoNames index. `aliases` are the lowercased primary
/// name, ASCII name and alternate names (Latin, Greek or Cyrillic script) that
/// the index can type as a LOCATION.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Place {
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub population: u64,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// The gazetteer that types single surfaces without context (brands, cities,
/// works, known people). `locations` is the hand-curated seed; `places` is the
/// real GeoNames index (cities500: populated places with population >= 5000,
/// plus capitals, CC BY 4.0). The typing mechanism is the same for both.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntityGazetteer {
    pub object_words: Vec<String>,
    /// Nouns that stay VISUAL_CONCEPT explicitly ("company").
    pub generic_visual_nouns: Vec<String>,
    pub company_suffixes: Vec<String>,
    pub brands: Vec<String>,
    pub locations: Vec<String>,
    /// GeoNames places; empty when the index is disabled by an override.
    #[serde(default)]
    pub places: Vec<Place>,
    /// Common words per language that are also place aliases ("corte",
    /// "come", "casa"). They are excluded from gazetteer typing in that
    /// language so a common noun never becomes a LOCATION.
    #[serde(default)]
    pub place_ambiguities: BTreeMap<String, Vec<String>>,
    pub works: Vec<String>,
    pub persons: Vec<String>,
    pub product_names: Vec<String>,
    pub product_substrings: Vec<String>,
}

/// Scaled-quantity vocabulary: scale words ("2 Millionen"), the English
/// singular chain ("2 million"), measurement units and currencies.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScaleWords {
    pub words: Vec<String>,
    pub english_scale_words: Vec<String>,
    pub unit_words: Vec<String>,
    pub currency_words: Vec<String>,
}

/// Contextual geographic cues (no gazetteer: syntax only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocationCues {
    pub context_prepositions: Vec<String>,
    pub genitive_prepositions: Vec<String>,
    pub container_nouns: Vec<String>,
    pub generic_location_nouns: Vec<String>,
    /// Locative prepositions that ground a bare prepositional phrase
    /// ("stadio di Torino" → located_in).
    pub sentence_locative_prepositions: Vec<String>,
}

/// Month names generated from CLDR (`lexicon/months.json`, synced by
/// `lexicon/tools/sync_cldr_months.py`): the ten supported languages' wide
/// month names in both CLDR contexts, plus documented legacy aliases. The
/// array order is the matching tie-break order.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Months {
    pub months: Vec<MonthEntry>,
    pub legacy_aliases: Vec<MonthAlias>,
}

/// One generated month name with its calendar number (1-12).
#[derive(Debug, Clone, Deserialize)]
pub struct MonthEntry {
    pub name: String,
    pub month: u8,
    #[serde(default)]
    pub locales: Vec<String>,
    #[serde(default)]
    pub contexts: Vec<String>,
}

/// A historical spelling kept for compatibility (see the sync tool).
#[derive(Debug, Clone, Deserialize)]
pub struct MonthAlias {
    pub name: String,
    pub month: u8,
    #[serde(default)]
    pub reason: String,
}

impl Months {
    /// The (name, calendar month) table in generated tie-break order, for
    /// date-span scanning.
    pub fn date_scan_table(&self) -> Vec<(String, u8)> {
        self.months
            .iter()
            .map(|entry| (entry.name.clone(), entry.month))
            .collect()
    }

    /// The names in generated tie-break order, for whole-token entity typing.
    pub fn typing_names(&self) -> Vec<String> {
        self.months.iter().map(|entry| entry.name.clone()).collect()
    }
}

/// Which grounded entity kinds may own a fact of one legal pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Person,
    Org,
    PersonOrOrg,
    /// Person, court or procedural role (GUP, prosecutor, defence).
    LegalActor,
}

/// The value side of one legal fact pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Verbatim date surface from the same sentence (required).
    Date,
    /// Verbatim numeric surface from the same sentence (required).
    Number,
    /// A second grounded entity, e.g. the partner or the owning organization.
    Entity,
    /// The marker itself: the fact is an event, not a quantity.
    Marker,
}

/// One legal/procedural fact pattern. Markers are matched case-insensitively
/// inside one canonical sentence; longer, more specific phrases must come
/// before their fragments so the first match carries the sharper meaning.
#[derive(Debug, Clone, Deserialize)]
pub struct LegalPattern {
    pub predicate: String,
    /// Canonical conclusion stored in `normalized_value`.
    pub normalized: String,
    pub markers: Vec<String>,
    pub subject: String,
    pub value: String,
}

impl LegalPattern {
    pub fn subject_kind(&self) -> SubjectKind {
        match self.subject.as_str() {
            "person" => SubjectKind::Person,
            "org" => SubjectKind::Org,
            "person_or_org" => SubjectKind::PersonOrOrg,
            _ => SubjectKind::LegalActor,
        }
    }

    pub fn value_kind(&self) -> ValueKind {
        match self.value.as_str() {
            "date" => ValueKind::Date,
            "number" => ValueKind::Number,
            "entity" => ValueKind::Entity,
            _ => ValueKind::Marker,
        }
    }
}

/// A curated legal actor surface (a court or procedural role a general
/// tokenizer cannot type reliably); only the semantic type is added.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityCue {
    pub cue: String,
    pub kind: String,
}

/// Legal/procedural extraction knowledge for ONE language. The pack is an
/// optional domain layer (`lexicon/legal/<lang>.json`): a language without a
/// pack produces no legal facts instead of borrowing another language's
/// markers. The core keeps working through the generic fact pipeline.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegalLexicon {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub patterns: Vec<LegalPattern>,
    /// Markers that make the age pattern skip a number ("16 anni di
    /// reclusione" is a sentence length, not an age).
    #[serde(default)]
    pub age_guard_markers: Vec<String>,
    #[serde(default)]
    pub entity_cues: Vec<EntityCue>,
    #[serde(default)]
    pub entity_alias_cues: Vec<EntityCue>,
    /// Sentences that talk about finality, not a new conviction.
    #[serde(default)]
    pub finality_markers: Vec<String>,
    /// Outcome words that qualify a dated micro summary as one step.
    #[serde(default)]
    pub outcome_markers: Vec<String>,
    #[serde(default)]
    pub comparison: LegalComparison,
    #[serde(default)]
    pub micro: MicroLexicon,
}

/// Cues and titles for the fact-comparison and step-sequence groups.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegalComparison {
    #[serde(default)]
    pub conviction_cues: Vec<String>,
    #[serde(default)]
    pub conviction_exclusions: Vec<String>,
    #[serde(default)]
    pub investigation_cues: Vec<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub sequence_title: String,
}

/// Micro-summary rendering for one language: templates and date suffixes.
/// A language without templates emits no micro summaries rather than Italian
/// text under a foreign language tag.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MicroLexicon {
    /// `"predicate|normalized"` → template with `{name}`, `{normalized}`,
    /// `{target}` and `{date}` placeholders.
    #[serde(default)]
    pub templates: BTreeMap<String, String>,
    #[serde(default)]
    pub date_year: String,
    #[serde(default)]
    pub date_surface: String,
    #[serde(default)]
    pub year_surface: String,
}

impl LegalLexicon {
    /// Template for one grounded fact, if this language renders it.
    pub fn micro_template(&self, predicate: &str, normalized: &str) -> Option<&str> {
        self.micro
            .templates
            .get(&format!("{predicate}|{normalized}"))
            .map(String::as_str)
    }
}

/// One ordered process phase (facility lifecycle).
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessPhase {
    pub order: usize,
    pub name: String,
    pub markers: Vec<String>,
}

/// Process/event knowledge: ordered lifecycle phases plus the event
/// predicates that qualify a sentence for a visual timeline.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProcessPhases {
    /// Ordered by `order`.
    pub phases: Vec<ProcessPhase>,
    pub event_predicate_markers: Vec<String>,
}

/// One comparison operator and the markers that express it.
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorCue {
    pub symbol: String,
    pub markers: Vec<String>,
}

/// Facility-fact knowledge (inauguration, capacity, operators, status).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FacilityLexicon {
    pub inauguration_markers: Vec<String>,
    pub capacity_markers: Vec<String>,
    pub capacity_unit_markers: Vec<String>,
    /// Ordered: the first matching operator wins.
    pub operators: Vec<OperatorCue>,
    pub status_uncertainty_cues: Vec<String>,
    pub status_negation_substrings: Vec<String>,
}

/// One explicit relation predicate and its multilingual markers.
#[derive(Debug, Clone, Deserialize)]
pub struct RelationMarkerGroup {
    pub predicate: String,
    pub markers: Vec<String>,
}

/// One entity-graph relation predicate (token-level markers).
#[derive(Debug, Clone, Deserialize)]
pub struct EntityRelationMarker {
    pub relation: String,
    pub markers: Vec<String>,
}

/// One comparison dimension and its multilingual keys.
#[derive(Debug, Clone, Deserialize)]
pub struct DimensionCue {
    pub name: String,
    pub markers: Vec<String>,
}

/// One number word and its numeric value ("tre" -> 3).
#[derive(Debug, Clone, Deserialize)]
pub struct NumberWord {
    pub value: i32,
    pub words: Vec<String>,
}

/// Every discourse/salience cue shared by phrase impact and scene
/// intelligence: one table instead of one list per module.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscourseCues {
    /// News-event salience cues per language; `default` covers every
    /// language without its own entry.
    pub content_cues: BTreeMap<String, Vec<String>>,
    pub filler_words: Vec<String>,
    pub filler_phrases: Vec<Vec<String>>,
    pub causal_after: Vec<Vec<String>>,
    pub causal_before: Vec<Vec<String>>,
    pub hypothetical: Vec<String>,
    pub attribution: Vec<String>,
    pub contribute: Vec<Vec<String>>,
    pub sequence: Vec<String>,
    pub comparison_signals: Vec<Vec<String>>,
    pub comparative_adj: Vec<String>,
    pub negation_tokens: Vec<String>,
    pub contrast_verbs: Vec<String>,
    /// Entity-graph relation predicates (token-level matching).
    pub entity_relation_markers: Vec<EntityRelationMarker>,
    /// Comparison dimensions ("investment", "cost", ...).
    pub dimensions: Vec<DimensionCue>,
    /// Separators that split a sentence around a causal/sequence marker.
    pub split_separators: Vec<String>,
    /// Relative time expressions ("anni dopo", "three years later", ...).
    pub relative_plural: Vec<Vec<String>>,
    pub relative_single: Vec<String>,
    pub relative_fixed: Vec<Vec<String>>,
    pub relative_expressions: Vec<String>,
    /// Year-unit words ("anno", "year", "jahr", "an").
    pub year_units: Vec<String>,
    /// Connectives between the two years of an interval ("dal 2020 al 2023").
    pub interval_connectives: Vec<String>,
    pub number_words: Vec<NumberWord>,
}

/// Explicit-relation and assertion-status vocabularies (one shared table).
/// The clause scoping and the negation/uncertainty readings are shared by
/// relation facts and legal facts so the two can never disagree again.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RelationCues {
    /// Separators that bound the clause a status is read from.
    pub clause_separators: Vec<String>,
    pub uncertainty_cues: Vec<String>,
    pub relation_markers: Vec<RelationMarkerGroup>,
    /// Negation cue words per language (`default` fallback), used to mark
    /// an assertion `negated` instead of dropping it.
    pub negation_cues: BTreeMap<String, Vec<String>>,
}

fn cues_for<'a>(table: &'a BTreeMap<String, Vec<String>>, language: &str) -> &'a [String] {
    let key = language.split(['-', '_']).next().unwrap_or("en");
    table
        .get(key)
        .or_else(|| table.get("default"))
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

impl DiscourseCues {
    /// Salience cues for a BCP-47 tag (falls back to the `default` table).
    pub fn content_cues_for(&self, language: &str) -> &[String] {
        cues_for(&self.content_cues, language)
    }
}

impl RelationCues {
    pub fn negation_cues_for(&self, language: &str) -> &[String] {
        cues_for(&self.negation_cues, language)
    }
}

/// The complete loaded lexicon.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    pub stopwords: Stopwords,
    pub objects: ObjectVocabulary,
    pub orthography: Orthography,
    pub language_profiles: LanguageProfiles,
    pub gazetteer: EntityGazetteer,
    /// Lowercased `gazetteer.locations` and place names/aliases, built once.
    pub place_index: std::collections::HashSet<String>,
    /// Lowercased ambiguous aliases per language, built once.
    pub place_ambiguities: BTreeMap<String, std::collections::HashSet<String>>,
    pub scale_words: ScaleWords,
    pub location_cues: LocationCues,
    pub process_phases: ProcessPhases,
    pub facility: FacilityLexicon,
    pub discourse: DiscourseCues,
    pub relation: RelationCues,
    /// Generated CLDR month table (`lexicon/months.json`).
    pub months: Months,
    /// `months` as (name, calendar month) in tie-break order.
    pub month_date_scan_table: Vec<(String, u8)>,
    /// `months` as names, for whole-token entity typing.
    pub month_typing_names: Vec<String>,
}

/// Loads one table: `$SEMANTIC_LEXICON_DIR/<file>` when present and valid,
/// otherwise the embedded default. A malformed override warns and falls back
/// so a broken deployment can never crash or silently change an analysis.
fn load_table<T: serde::de::DeserializeOwned>(file: &str, embedded: &'static str) -> T {
    if let Ok(dir) = std::env::var(LEXICON_DIR_ENV) {
        let path = std::path::Path::new(&dir).join(file);
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str(&text) {
                Ok(table) => return table,
                Err(error) => {
                    eprintln!("semantic-core lexicon: ignoring {path:?}: {error}");
                }
            }
        }
    }
    serde_json::from_str(embedded)
        .unwrap_or_else(|error| panic!("embedded lexicon table {file} is invalid: {error}"))
}

/// Builds the per-language ambiguity lookup. A word listed for a language is
/// never typed as a place in that language, even though the index contains it.
fn build_place_ambiguities(
    gazetteer: &EntityGazetteer,
) -> BTreeMap<String, std::collections::HashSet<String>> {
    gazetteer
        .place_ambiguities
        .iter()
        .map(|(language, words)| {
            (
                language.clone(),
                words.iter().map(|word| word.to_lowercase()).collect(),
            )
        })
        .collect()
}

/// Builds the O(1) place lookup from the curated seeds and the GeoNames
/// index. Aliases are already lowercased by the generator; the extra
/// conversion keeps a hand-written override safe.
fn build_place_index(gazetteer: &EntityGazetteer) -> std::collections::HashSet<String> {
    let mut index = std::collections::HashSet::with_capacity(
        gazetteer.locations.len() + gazetteer.places.len() * 6,
    );
    index.extend(
        gazetteer
            .locations
            .iter()
            .map(|location| location.to_lowercase()),
    );
    for place in &gazetteer.places {
        index.insert(place.name.to_lowercase());
        index.extend(place.aliases.iter().map(|alias| alias.to_lowercase()));
    }
    index
}

impl Lexicon {
    /// Builds the lexicon once: embedded defaults with any runtime override.
    pub fn global() -> &'static Lexicon {
        static LEXICON: OnceLock<Lexicon> = OnceLock::new();
        LEXICON.get_or_init(|| {
            let gazetteer: EntityGazetteer = load_table(
                "entity_gazetteer.json",
                include_str!("../../lexicon/entity_gazetteer.json"),
            );
            let place_index = build_place_index(&gazetteer);
            let place_ambiguities = build_place_ambiguities(&gazetteer);
            let months: Months =
                load_table("months.json", include_str!("../../lexicon/months.json"));
            let month_date_scan_table = months.date_scan_table();
            let month_typing_names = months.typing_names();
            Lexicon {
                stopwords: load_table(
                    "stopwords.json",
                    include_str!("../../lexicon/stopwords.json"),
                ),
                objects: load_table(
                    "object_vocabulary.json",
                    include_str!("../../lexicon/object_vocabulary.json"),
                ),
                orthography: load_table(
                    "orthography.json",
                    include_str!("../../lexicon/orthography.json"),
                ),
                language_profiles: load_table(
                    "language_profiles.json",
                    include_str!("../../lexicon/language_profiles.json"),
                ),
                gazetteer,
                place_index,
                place_ambiguities,
                scale_words: load_table(
                    "scale_words.json",
                    include_str!("../../lexicon/scale_words.json"),
                ),
                location_cues: load_table(
                    "location_cues.json",
                    include_str!("../../lexicon/location_cues.json"),
                ),
                process_phases: load_table(
                    "process_phases.json",
                    include_str!("../../lexicon/process_phases.json"),
                ),
                facility: load_table(
                    "facility_facts.json",
                    include_str!("../../lexicon/facility_facts.json"),
                ),
                discourse: load_table(
                    "discourse_cues.json",
                    include_str!("../../lexicon/discourse_cues.json"),
                ),
                relation: load_table(
                    "relation_cues.json",
                    include_str!("../../lexicon/relation_cues.json"),
                ),
                months,
                month_date_scan_table,
                month_typing_names,
            }
        })
    }

    /// True when a capitalized surface is a known place. The capitalization
    /// guard keeps lowercase common nouns that collide with a GeoNames alias
    /// ("casa" -> Chas, "mare" -> Male) out of the LOCATION typing.
    pub fn is_place(&self, surface: &str, language: &str) -> bool {
        // The capitalization guard only means something in a cased script.
        // Han/Japanese/Arabic/Thai have no uppercase initial, so requiring one
        // would make every proper name in those scripts invisible to the
        // index. German keeps the guard: it is cased, even though every noun
        // is capitalized.
        if self.language_profiles.is_cased(language)
            && !surface.chars().next().is_some_and(char::is_uppercase)
        {
            return false;
        }
        let lower = surface.to_lowercase();
        let key = language.split(['-', '_']).next().unwrap_or("");
        if self
            .place_ambiguities
            .get(key)
            .is_some_and(|ambiguous| ambiguous.contains(&lower))
        {
            return false;
        }
        self.place_index.contains(&lower)
    }

    /// Object terms of one BCP-47 language; empty for unsupported languages.
    pub fn object_terms(&self, language: &str) -> &[String] {
        let key = language.split(['-', '_']).next().unwrap_or("");
        self.objects
            .terms
            .get(key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Convenience accessor for [`Lexicon::global`].
pub fn lexicon() -> &'static Lexicon {
    Lexicon::global()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_tables_load_with_the_documented_shape() {
        let lexicon = Lexicon::global();
        assert!(!lexicon.stopwords.stop_words.is_empty());
        assert!(lexicon
            .stopwords
            .generic_phrases
            .iter()
            .any(|p| p == "imagine"));
        assert_eq!(lexicon.object_terms("en").len(), 24);
        assert_eq!(lexicon.object_terms("it").len(), 20);
        // BCP-47 subtags resolve to the base language.
        assert_eq!(
            lexicon.object_terms("pt-BR").len(),
            lexicon.object_terms("pt").len()
        );
        assert!(lexicon.object_terms("xx").is_empty());
        assert_eq!(lexicon.process_phases.phases.len(), 4);
        assert_eq!(lexicon.relation.relation_markers.len(), 7);
    }

    #[test]
    fn geonames_places_are_indexed_with_a_capitalization_guard() {
        let lexicon = Lexicon::global();
        assert!(lexicon.is_place("Pavia", "it"));
        assert!(lexicon.is_place("Garlasco", "it"));
        assert!(lexicon.is_place("Vigevano", "it"));
        // A caseless script has no uppercase initial: the index must still
        // type the place.
        assert!(lexicon.is_place("北京", "zh"));
        assert!(lexicon.is_place("東京", "ja"));
        assert!(lexicon.is_place("서울", "ko"));
        assert!(lexicon.is_place("القاهرة", "ar"));
        assert!(lexicon.is_place("मुंबई", "hi"));
        // German is cased even though nouns are capitalized: the lowercase
        // guard still applies.
        assert!(lexicon.is_place("Berlin", "de"));
        assert!(!lexicon.is_place("berlin", "de"));
        // Lowercase common nouns that collide with a GeoNames alias stay out.
        assert!(!lexicon.is_place("casa", "it"));
        assert!(!lexicon.is_place("mare", "it"));
        assert!(!lexicon.is_place("pavia", "it"));
        // "Corte" is both a French town and the Italian common noun "court".
        assert!(!lexicon.is_place("Corte", "it"));
        assert!(lexicon.is_place("Corte", "en"));
    }

    #[test]
    fn language_profiles_own_the_per_language_conventions() {
        let profiles = &Lexicon::global().language_profiles;
        // Treebank: BCP-47 subtags resolve to the base language, unknown
        // languages have none and never borrow `default`.
        assert_eq!(profiles.treebank("it"), Some("italian-isdt"));
        assert_eq!(profiles.treebank("pt-BR"), Some("portuguese-gsd"));
        assert_eq!(profiles.treebank(" DE "), Some("german-gsd"));
        assert_eq!(profiles.treebank("zh"), Some("chinese-gsdsimp"));
        // A language without a certified model never borrows another one.
        assert_eq!(profiles.treebank("ko"), None);
        assert_eq!(profiles.treebank(""), None);

        // Date order and numeric separators are facts, not a `language == "en"`.
        assert_eq!(profiles.date_order("en"), DateOrder::MonthFirst);
        assert_eq!(profiles.date_order("it"), DateOrder::DayFirst);
        assert_eq!(profiles.date_order("xx"), DateOrder::DayFirst);
        assert_eq!(profiles.decimal_separator("en"), '.');
        assert_eq!(profiles.thousands_separator("en"), ',');
        assert_eq!(profiles.decimal_separator("it"), ',');
        assert_eq!(profiles.thousands_separator("it"), '.');
        assert_eq!(profiles.thousands_separator("xx"), '.');

        // Verbal morphology is certified per language, never inferred from a
        // suffix list that happens to be non-empty.
        assert!(profiles.verb_filter("it"));
        assert!(!profiles.verb_filter("de"));
        assert_eq!(
            profiles.infinitive_suffixes("it"),
            ["re".to_string()].as_slice()
        );
        assert!(profiles.infinitive_suffixes("de").is_empty());
    }

    #[test]
    fn writing_systems_decide_whether_case_is_name_evidence() {
        let profiles = &Lexicon::global().language_profiles;
        // Cased scripts.
        assert!(profiles.is_cased("en"));
        assert!(profiles.is_cased("ru"));
        // Caseless scripts have no uppercase initial to test.
        assert!(!profiles.is_cased("zh"));
        assert!(!profiles.is_cased("ja"));
        assert!(!profiles.is_cased("ar"));
        assert!(!profiles.is_cased("th"));
        // Unknown languages keep the historical cased behavior.
        assert!(profiles.is_cased("xx"));
        // German capitalizes common nouns: the case test carries no signal.
        assert!(profiles.is_cased("de"));
        assert!(profiles.capitalized_nouns("de"));
        assert!(!profiles.uppercase_is_name_evidence("de"));
        assert!(profiles.uppercase_is_name_evidence("it"));
        assert!(!profiles.uppercase_is_name_evidence("zh"));
    }

    #[test]
    fn embedded_language_profiles_pass_their_own_validation() {
        let problems = Lexicon::global().language_profiles.validate();
        assert!(
            problems.is_empty(),
            "invalid language profile table: {problems:#?}"
        );
    }

    #[test]
    fn profile_validation_names_each_broken_entry() {
        let broken: LanguageProfiles = serde_json::from_value(serde_json::json!({
            "profiles": {
                "de": { "script": "klingon" },
                "it": { "verb_filter": true, "script": "latin" },
                "xx": { "treebank": "  " }
            },
            "scripts": { "latin": { "cased": true }, "broken": {} }
        }))
        .expect("the shape parses so validation can name the problems");
        let joined = broken.validate().join("\n");
        assert!(joined.contains("unknown script \"klingon\""), "{joined}");
        assert!(joined.contains("missing mandatory `script`"), "{joined}");
        assert!(joined.contains("empty `treebank`"), "{joined}");
        assert!(
            joined.contains("needs at least one `infinitive_suffixes`"),
            "{joined}"
        );
        assert!(joined.contains("missing mandatory `cased`"), "{joined}");
    }

    #[test]
    fn orthography_tables_supply_abbreviations_and_eras() {
        let orthography = &Lexicon::global().orthography;
        let italian: Vec<&str> = orthography.abbreviations.for_language("it").collect();
        assert!(italian.contains(&"dott."));
        assert!(
            italian.contains(&"etc."),
            "common abbreviations apply to every language"
        );
        let german: Vec<&str> = orthography.abbreviations.for_language("de").collect();
        assert!(german.contains(&"z.b."));
        assert!(!german.contains(&"dott."));
        assert!(orthography
            .era_markers
            .iter()
            .any(|marker| marker.marker == "b.c." && marker.era == "bce"));
    }

    #[test]
    fn discourse_cues_fall_back_to_the_default_language() {
        let lexicon = Lexicon::global();
        assert_eq!(
            lexicon.discourse.content_cues_for("it"),
            lexicon.discourse.content_cues.get("it").unwrap().as_slice()
        );
        let fallback = lexicon.discourse.content_cues_for("xx");
        let default = lexicon.discourse.content_cues.get("default").unwrap();
        assert_eq!(fallback, default.as_slice());
    }

    #[test]
    fn order_sensitve_tables_keep_their_data_order() {
        // Longer markers must precede their fragments ("a causa di" before
        // "a causa"), so the first match carries the sharper meaning.
        let lexicon = Lexicon::global();
        assert_eq!(lexicon.discourse.causal_after[0], ["a", "causa", "di"]);
        let legal = crate::legal_packs::get("it").expect("Italian legal pack");
        assert_eq!(legal.patterns[0].normalized, "not_convicted");
    }
}
