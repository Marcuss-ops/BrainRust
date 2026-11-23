//! Event trigger lexicons.

use std::collections::BTreeMap;

use serde::Deserialize;

const EN: &str = include_str!("../../../lexicon/events/en.json");
const IT: &str = include_str!("../../../lexicon/events/it.json");
const ES: &str = include_str!("../../../lexicon/events/es.json");
const FR: &str = include_str!("../../../lexicon/events/fr.json");
const PT: &str = include_str!("../../../lexicon/events/pt.json");
const DE: &str = include_str!("../../../lexicon/events/de.json");
const NL: &str = include_str!("../../../lexicon/events/nl.json");
const ID: &str = include_str!("../../../lexicon/events/id.json");
const PL: &str = include_str!("../../../lexicon/events/pl.json");
const RU: &str = include_str!("../../../lexicon/events/ru.json");
const TR: &str = include_str!("../../../lexicon/events/tr.json");

#[derive(Deserialize)]
pub(super) struct TriggerFile {
    language: String,
    pub(super) event_types: BTreeMap<String, Vec<TriggerEntry>>,
}

#[derive(Deserialize)]
pub(super) struct TriggerEntry {
    #[serde(default)]
    pub(super) surface: Option<String>,
    pub(super) lemma: String,
    pub(super) voice: String,
    pub(super) roles: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) pro_drop_roles: Vec<String>,
}

pub(super) fn lexicon(language: &str) -> Option<TriggerFile> {
    let base = language.split('-').next().unwrap_or(language);
    let data = match base {
        "en" => EN,
        "it" => IT,
        "es" => ES,
        "fr" => FR,
        "pt" => PT,
        "de" => DE,
        "nl" => NL,
        "id" => ID,
        "pl" => PL,
        "ru" => RU,
        "tr" => TR,
        _ => return None,
    };
    let file: TriggerFile = serde_json::from_str(data).ok()?;
    (file.language == base).then_some(file)
}
