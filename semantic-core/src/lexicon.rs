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
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct GivenNameInventory {
    languages: BTreeMap<String, GivenNameGenderLists>,
}

#[derive(serde::Deserialize)]
struct GivenNameGenderLists {
    feminine: Vec<String>,
    masculine: Vec<String>,
    #[serde(default)]
    unknown: Vec<String>,
}

fn given_name_inventory() -> &'static GivenNameInventory {
    static INVENTORY: OnceLock<GivenNameInventory> = OnceLock::new();
    INVENTORY.get_or_init(|| {
        serde_json::from_str(include_str!("../../lexicon/given_names.json"))
            .expect("embedded given-name inventory must be valid")
    })
}

/// Gender evidence for the first token in a grounded full name. Names absent
/// from the curated language inventory stay `unknown` and remain compatible.
pub fn given_name_gender(language: &str, full_name: &str) -> &'static str {
    let language = language.split(['-', '_']).next().unwrap_or(language);
    let given_name = full_name.split_whitespace().next().unwrap_or("");
    let Some(lists) = given_name_inventory().languages.get(language) else {
        return "unknown";
    };
    let feminine = lists
        .feminine
        .iter()
        .any(|name| name.eq_ignore_ascii_case(given_name));
    let masculine = lists
        .masculine
        .iter()
        .any(|name| name.eq_ignore_ascii_case(given_name));
    let unknown = lists
        .unknown
        .iter()
        .any(|name| name.eq_ignore_ascii_case(given_name));
    match (feminine, masculine, unknown) {
        (true, false, false) => "feminine",
        (false, true, false) => "masculine",
        _ => "unknown",
    }
}

/// Whether a name opener has curated given-name evidence in its language.
pub fn is_curated_given_name(language: &str, surface: &str) -> bool {
    let language = language.split(['-', '_']).next().unwrap_or(language);
    let given_name = surface.split_whitespace().next().unwrap_or("");
    given_name_inventory()
        .languages
        .get(language)
        .is_some_and(|lists| {
            lists
                .feminine
                .iter()
                .chain(&lists.masculine)
                .chain(&lists.unknown)
                .any(|name| name.eq_ignore_ascii_case(given_name))
        })
}

/// Version of the lexicon tables. Bump whenever a table changes semantics.
pub const LEXICON_VERSION: &str = "lexicon.v8";

/// Environment variable naming a directory of overriding table files.
pub const LEXICON_DIR_ENV: &str = "SEMANTIC_LEXICON_DIR";

/// Tables loaded from `$SEMANTIC_LEXICON_DIR` instead of the embedded copy.
static OVERRIDES_LOADED: AtomicUsize = AtomicUsize::new(0);
/// Override files that were present but unreadable/malformed, and therefore
/// fell back to the embedded copy.
static OVERRIDES_FAILED: AtomicUsize = AtomicUsize::new(0);
/// The override directory as it was when the lexicon was first loaded; the
/// environment cannot change the provenance of an already-loaded lexicon.
static OVERRIDE_DIR: OnceLock<Option<String>> = OnceLock::new();

fn override_dir() -> Option<&'static str> {
    OVERRIDE_DIR
        .get_or_init(|| {
            std::env::var(LEXICON_DIR_ENV)
                .ok()
                .filter(|dir| !dir.trim().is_empty())
        })
        .as_deref()
}

/// Content fingerprint of the effective `$SEMANTIC_LEXICON_DIR` override.
///
/// The embedded fingerprint cannot cover a directory that replaces tables at
/// runtime, so a persistent cache key must include this value as well. The
/// result is computed once per process: the lexicon is loaded once, and a file
/// change after startup cannot affect the running analysis anyway. `None`
/// means the embedded tables are in use.
pub fn override_content_fingerprint() -> Option<String> {
    static FINGERPRINT: OnceLock<Option<String>> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| override_dir().map(|dir| directory_fingerprint(Path::new(dir))))
        .clone()
}

/// SHA-256 over the relative names and bytes of every file under `root`.
/// Names are sorted and normalized to `/`, so the digest is independent of
/// insertion order and of the deployment path. A read error is hashed instead
/// of being silently skipped: an unreadable override must not share the
/// embedded identity.
fn directory_fingerprint(root: &Path) -> String {
    fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                collect(root, &path, files);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, path));
            }
        }
    }

    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    hasher.update(b"semantic-lexicon-override");
    for (name, path) in files {
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        match std::fs::read(&path) {
            Ok(bytes) => {
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(error) => {
                hasher.update(b"error");
                hasher.update(error.to_string().as_bytes());
            }
        }
    }
    crate::text::lowercase_hex(&hasher.finalize())
}

/// Where the running lexicon actually came from. A deployment that sets
/// `SEMANTIC_LEXICON_DIR` but ships a broken table used to leave only a stderr
/// warning; exposing this in `health` makes the fallback observable instead of
/// silent. `source` is `embedded` when no table was overridden, `override`
/// when every attempted table was valid, and `mixed` when some tables were
/// overridden and others fell back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconProvenance {
    pub version: &'static str,
    pub override_dir: Option<String>,
    pub overrides_loaded: usize,
    pub overrides_failed: usize,
}

impl LexiconProvenance {
    pub fn source(&self) -> &'static str {
        match (self.overrides_loaded > 0, self.overrides_failed > 0) {
            (true, true) => "mixed",
            (true, false) => "override",
            _ => "embedded",
        }
    }
}

/// Forces the lexicon to load (once) and reports its provenance.
pub fn provenance() -> LexiconProvenance {
    let _ = Lexicon::global();
    LexiconProvenance {
        version: LEXICON_VERSION,
        override_dir: override_dir().map(str::to_string),
        overrides_loaded: OVERRIDES_LOADED.load(Ordering::Relaxed),
        overrides_failed: OVERRIDES_FAILED.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct EntityGazetteerDelta {
    #[serde(default)]
    remove_brands: Vec<String>,
    #[serde(default)]
    product_names: Vec<String>,
    #[serde(default)]
    event_head_words: Vec<String>,
    /// Curated place surfaces the generated GeoNames table does not carry:
    /// exonyms (Gerusalemme for Jerusalem) and region names that are not
    /// populated places (Vicino Oriente). The ready-made `locations` seed
    /// works the same way, but it lives in the generated table, so the delta
    /// keeps these additions reviewable instead of rewriting the dump.
    #[serde(default)]
    locations: Vec<String>,
    #[serde(default)]
    typed_entities: BTreeMap<String, Vec<String>>,
}

fn extend_unique(target: &mut Vec<String>, additions: Vec<String>) {
    for value in additions {
        if !target
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&value))
        {
            target.push(value);
        }
    }
}

// ── Table shapes ────────────────────────────────────────────────────

mod cues;
mod legal;
mod orthography;
mod profiles;
mod vocabulary;

pub use cues::{
    AttributeExtractorDef, AttributePredicateDef, AttributePredicates, DimensionCue, DiscourseCues,
    EntityRelationMarker, FacilityLexicon, IntroAgendaLexicon, NumberWord, OperatorCue,
    ProcessPhase, ProcessPhases, RelationCues, RelationMarkerGroup,
};
pub use legal::{
    EntityCue, LegalComparison, LegalLexicon, LegalPattern, MicroLexicon, SubjectKind, ValueKind,
};
pub use orthography::{Abbreviations, EraMarker, Orthography};
pub use profiles::{DateOrder, LanguageProfile, LanguageProfiles, ScriptProfile, Stopwords};
pub use vocabulary::{
    EntityGazetteer, LocationCues, MonthAlias, MonthEntry, Months, ObjectVocabulary, OrderedSteps,
    Place, PlaceInflections, ScaleWords,
};

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
    /// Lowercased alias-only names of places below the minor threshold. They
    /// stay out of LOCATION typing: a village alias is not stronger evidence
    /// than the capitalized-token name rule ("João Ribeiro" is a person).
    pub minor_place_aliases: std::collections::HashSet<String>,
    /// Lowercased ambiguous aliases per language, built once.
    pub place_ambiguities: BTreeMap<String, std::collections::HashSet<String>>,
    /// Locale-scoped inflected place surfaces and their canonical targets.
    pub place_inflections: PlaceInflections,
    /// Language -> lowercased inflected surface, built once.
    pub inflected_place_index: BTreeMap<String, std::collections::HashSet<String>>,
    /// Language -> lowercased inflected surface -> canonical place name.
    pub place_canonicals: BTreeMap<String, BTreeMap<String, String>>,
    pub ordered_steps: OrderedSteps,
    pub scale_words: ScaleWords,
    pub location_cues: LocationCues,
    pub process_phases: ProcessPhases,
    pub facility: FacilityLexicon,
    /// Canonical attribute predicate ontology and its localized lemmas.
    pub attribute_predicates: AttributePredicates,
    /// Minimal conclusion markers used to keep spoilers out of the intro.
    pub intro_agenda: IntroAgendaLexicon,
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
/// otherwise the embedded default. A missing or malformed override warns and
/// falls back so a broken deployment can never crash or silently change an
/// analysis.
fn load_table<T: serde::de::DeserializeOwned>(file: &str, embedded: &'static str) -> T {
    if let Some(dir) = override_dir() {
        let path = std::path::Path::new(dir).join(file);
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(table) => {
                    OVERRIDES_LOADED.fetch_add(1, Ordering::Relaxed);
                    return table;
                }
                Err(error) => {
                    OVERRIDES_FAILED.fetch_add(1, Ordering::Relaxed);
                    eprintln!("semantic-core lexicon: ignoring {path:?}: {error}");
                }
            },
            Err(error) => {
                OVERRIDES_FAILED.fetch_add(1, Ordering::Relaxed);
                eprintln!("semantic-core lexicon: using embedded default for {path:?}: {error}");
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
/// Places below this population are only recognized through their primary
/// name or the curated seed, never through a bare alias: small-town aliases
/// collide with person names ("João Ribeiro").
const MINOR_PLACE_ALIAS_POPULATION: u64 = 50_000;

type InflectedPlaceIndex = BTreeMap<String, std::collections::HashSet<String>>;
type PlaceCanonicals = BTreeMap<String, BTreeMap<String, String>>;

/// Builds the language-scoped alias lookup and its canonical target. The
/// mapping is exact and data-driven: a surface is never fuzzy-matched against
/// the 69k-place index.
fn build_inflected_place_index(
    place_inflections: &PlaceInflections,
) -> (InflectedPlaceIndex, PlaceCanonicals) {
    let mut index: BTreeMap<String, std::collections::HashSet<String>> = BTreeMap::new();
    let mut canonicals: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (language, table) in &place_inflections.languages {
        let key = language.split(['-', '_']).next().unwrap_or("").to_string();
        for (canonical, forms) in &table.places {
            for form in forms {
                let lower = form.to_lowercase();
                index.entry(key.clone()).or_default().insert(lower.clone());
                canonicals
                    .entry(key.clone())
                    .or_default()
                    .insert(lower, canonical.clone());
            }
        }
    }
    (index, canonicals)
}

fn build_place_index(
    gazetteer: &EntityGazetteer,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let mut index = std::collections::HashSet::with_capacity(
        gazetteer.locations.len() + gazetteer.places.len() * 6,
    );
    let mut primary = std::collections::HashSet::new();
    let mut major_aliases = std::collections::HashSet::new();
    let mut minor_aliases = std::collections::HashSet::new();
    for location in &gazetteer.locations {
        let lower = location.to_lowercase();
        index.insert(lower.clone());
        primary.insert(lower);
    }
    for place in &gazetteer.places {
        let name = place.name.to_lowercase();
        index.insert(name.clone());
        primary.insert(name);
        for alias in &place.aliases {
            let alias = alias.to_lowercase();
            index.insert(alias.clone());
            if place.population < MINOR_PLACE_ALIAS_POPULATION {
                minor_aliases.insert(alias);
            } else {
                major_aliases.insert(alias);
            }
        }
    }
    // A name that any place uses as its primary name, or an alias shared with
    // a major place, is not a minor-only alias.
    minor_aliases.retain(|alias| !primary.contains(alias) && !major_aliases.contains(alias));
    (index, minor_aliases)
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
            let mut gazetteer = gazetteer;
            // Keep product and organization curation in a small, reviewable
            // delta instead of rewriting the generated GeoNames table.
            let entity_delta: EntityGazetteerDelta =
                serde_json::from_str(include_str!("../../lexicon/entity_gazetteer_delta.json"))
                    .expect("embedded entity gazetteer delta must be valid JSON");
            gazetteer
                .brands
                .retain(|brand| !entity_delta.remove_brands.contains(brand));
            extend_unique(&mut gazetteer.product_names, entity_delta.product_names);
            extend_unique(
                &mut gazetteer.event_head_words,
                entity_delta.event_head_words,
            );
            extend_unique(&mut gazetteer.locations, entity_delta.locations);
            for (kind, names) in entity_delta.typed_entities {
                extend_unique(gazetteer.typed_entities.entry(kind).or_default(), names);
            }
            let (place_index, minor_place_aliases) = build_place_index(&gazetteer);
            let place_ambiguities = build_place_ambiguities(&gazetteer);
            let place_inflections: PlaceInflections = load_table(
                "place_inflections.json",
                include_str!("../../lexicon/place_inflections.json"),
            );
            let (inflected_place_index, place_canonicals) =
                build_inflected_place_index(&place_inflections);
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
                minor_place_aliases,
                place_ambiguities,
                place_inflections,
                inflected_place_index,
                place_canonicals,
                ordered_steps: load_table(
                    "ordered_steps.json",
                    include_str!("../../lexicon/ordered_steps.json"),
                ),
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
                attribute_predicates: load_table(
                    "attribute_predicates.json",
                    include_str!("../../lexicon/attribute_predicates.json"),
                ),
                intro_agenda: load_table(
                    "intro_agenda.json",
                    include_str!("../../lexicon/intro_agenda.json"),
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
        // The curated location seed contains country and city names that are
        // deliberately outside the GeoNames populated-place dump (a country is
        // ADM0, not feature class P). They are still places: the same list is
        // already trusted by the visual typer, so the predicate must agree.
        if self
            .gazetteer
            .locations
            .iter()
            .any(|name| name.eq_ignore_ascii_case(surface) || name.eq_ignore_ascii_case(&lower))
        {
            return true;
        }
        let key = language.split(['-', '_']).next().unwrap_or("");
        // A function word of the text's own language is not a place, whatever
        // the index says: the index is matched case-insensitively, so a
        // sentence-initial function word that also names a town elsewhere
        // would be typed as that town (Indonesian "Bukan" means "not" and is
        // also a city in Iran). Only a single-token surface is refused, so a
        // compound name keeps its open-class words ("Great Smoky Mountains").
        if !lower.contains(char::is_whitespace)
            && crate::language_packs::get(language)
                .is_some_and(|pack| pack.is_function_word(&lower))
        {
            return false;
        }
        if self
            .place_ambiguities
            .get(key)
            .is_some_and(|ambiguous| ambiguous.contains(&lower))
        {
            return false;
        }
        if self.minor_place_aliases.contains(&lower) {
            return false;
        }
        // A named negative word with the same ending is never a place in its
        // own language, even when the global index would accept it.
        if self
            .place_inflections
            .languages
            .get(key)
            .is_some_and(|table| {
                table
                    .blocklist
                    .iter()
                    .any(|word| word.to_lowercase() == lower)
            })
        {
            return false;
        }
        if self
            .inflected_place_index
            .get(key)
            .is_some_and(|forms| forms.contains(&lower))
        {
            return true;
        }
        // A month name of the current language is a calendar word, not a
        // place. The closed month table (CLDR-derived `months.json`) decides
        // this — the same seam as the ambiguity table for words like
        // "March", but derived from a closed class instead of typed by hand.
        if self
            .months
            .months
            .iter()
            .any(|entry| entry.name == lower && entry.locales.iter().any(|locale| locale == key))
        {
            return false;
        }
        self.place_index.contains(&lower)
    }

    /// Canonical place name for a locale-scoped inflected surface. This is
    /// exact alias resolution, never a fuzzy match against the gazetteer.
    pub fn canonical_place(&self, surface: &str, language: &str) -> Option<&str> {
        let key = language.split(['-', '_']).next().unwrap_or("");
        self.place_canonicals
            .get(key)?
            .get(&surface.to_lowercase())
            .map(String::as_str)
    }

    /// Canonical surface for query composition when the language's documented
    /// policy uses the nominative apposition (Polish). `None` keeps the source
    /// surface (Russian).
    pub fn canonical_query_place(&self, surface: &str, language: &str) -> Option<&str> {
        let key = language.split(['-', '_']).next().unwrap_or("");
        self.place_inflections
            .languages
            .get(key)
            .filter(|table| table.query_form == "canonical")?;
        self.canonical_place(surface, language)
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
mod tests;
