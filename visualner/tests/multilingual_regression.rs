//! Multilingual regression suite: 10 sentences for each of the nine languages
//! in config/multilingual.yaml (it, en, de, es, fr, ru, pl, tr, id).
//!
//! Each sentence is sent to the real `visualner` binary twice: once with the
//! UDPipe models (`VISUALNER_MODEL_DIR`, or the default model directory) and
//! once with an empty model directory, which exercises the stopword fallback.
//! The expected chart values are fixed in this file, so both modes must
//! produce exactly the same numbers, and the run must not crash or report an
//! error. Proper nouns are required as entities only in model mode, because
//! the stopword fallback does not tag them.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// (language, sentence, expected chart values in order, proper nouns that must
/// appear as entities in model mode).
type Case = (
    &'static str,
    &'static str,
    &'static [f64],
    &'static [&'static str],
);

const CASES: &[Case] = &[
    // it
    (
        "it",
        "Il fatturato è cresciuto del 25% a Roma.",
        &[25.0],
        &["Roma"],
    ),
    (
        "it",
        "Le vendite sono salite del 12,5% in Italia.",
        &[12.5],
        &["Italia"],
    ),
    (
        "it",
        "Milano ha registrato un calo del 7% nel 2024.",
        &[7.0],
        &["Milano"],
    ),
    (
        "it",
        "Il progetto ha raggiunto 2 milioni di utenti.",
        &[],
        &[],
    ),
    (
        "it",
        "Torino ospita un festival con il 40% di visitatori stranieri.",
        &[40.0],
        &["Torino"],
    ),
    (
        "it",
        "Napoli ha speso il 15% del budget in pubblicità.",
        &[15.0],
        &["Napoli"],
    ),
    (
        "it",
        "La Sicilia cresce del 3% ogni anno.",
        &[3.0],
        &["Sicilia"],
    ),
    (
        "it",
        "Firenze ha ospitato 500 mila turisti nel 2025.",
        &[],
        &["Firenze"],
    ),
    (
        "it",
        "Venezia ha ridotto le emissioni del 30% in dieci anni.",
        &[30.0],
        &["Venezia"],
    ),
    (
        "it",
        "Bologna ha aumentato il personale del 18% quest'anno.",
        &[18.0],
        &["Bologna"],
    ),
    // en
    (
        "en",
        "Revenue grew 25% in London this year.",
        &[25.0],
        &["London"],
    ),
    (
        "en",
        "Sales rose 12.5% across Europe.",
        &[12.5],
        &["Europe"],
    ),
    (
        "en",
        "Berlin reported a 7% drop in 2024.",
        &[7.0],
        &["Berlin"],
    ),
    ("en", "The project reached 2 million users.", &[], &[]),
    (
        "en",
        "Madrid hosts a festival with 40% foreign visitors.",
        &[40.0],
        &["Madrid"],
    ),
    (
        "en",
        "Paris spent 15% of its budget on advertising.",
        &[15.0],
        &["Paris"],
    ),
    ("en", "Tokyo grows 3% every year.", &[3.0], &["Tokyo"]),
    (
        "en",
        "Chicago welcomed 500 thousand tourists in 2025.",
        &[],
        &["Chicago"],
    ),
    (
        "en",
        "Amsterdam cut emissions by 30% in ten years.",
        &[30.0],
        &["Amsterdam"],
    ),
    (
        "en",
        "Dublin increased staff by 18% this year.",
        &[18.0],
        &["Dublin"],
    ),
    // de
    (
        "de",
        "Der Umsatz stieg um 25% in Berlin.",
        &[25.0],
        &["Berlin"],
    ),
    (
        "de",
        "Die Verkäufe stiegen um 12,5% in Europa.",
        &[12.5],
        &["Europa"],
    ),
    (
        "de",
        "München meldete einen Rückgang von 7% im Jahr 2024.",
        &[7.0],
        &["München"],
    ),
    ("de", "Das Projekt erreichte 2 Millionen Nutzer.", &[], &[]),
    (
        "de",
        "Hamburg hat ein Festival mit 40% ausländischen Besuchern.",
        &[40.0],
        &["Hamburg"],
    ),
    (
        "de",
        "Köln gab 15% des Budgets für Werbung aus.",
        &[15.0],
        &["Köln"],
    ),
    ("de", "Wien wächst jedes Jahr um 3%.", &[3.0], &["Wien"]),
    (
        "de",
        "Frankfurt begrüßte 500 Tausend Touristen im Jahr 2025.",
        &[],
        &["Frankfurt"],
    ),
    (
        "de",
        "Amsterdam senkte die Emissionen um 30% in zehn Jahren.",
        &[30.0],
        &["Amsterdam"],
    ),
    (
        "de",
        "Dresden erhöhte das Personal um 18% in diesem Jahr.",
        &[18.0],
        &["Dresden"],
    ),
    // es
    (
        "es",
        "Los ingresos crecieron un 25% en Madrid.",
        &[25.0],
        &["Madrid"],
    ),
    (
        "es",
        "Las ventas subieron un 12,5% en Europa.",
        &[12.5],
        &["Europa"],
    ),
    (
        "es",
        "Sevilla registró una caída del 7% en 2024.",
        &[7.0],
        &["Sevilla"],
    ),
    (
        "es",
        "El proyecto alcanzó 2 millones de usuarios.",
        &[],
        &[],
    ),
    (
        "es",
        "Valencia organiza un festival con 40% de visitantes extranjeros.",
        &[40.0],
        &["Valencia"],
    ),
    (
        "es",
        "Bilbao gastó el 15% del presupuesto en publicidad.",
        &[15.0],
        &["Bilbao"],
    ),
    ("es", "Lisboa crece un 3% cada año.", &[3.0], &["Lisboa"]),
    (
        "es",
        "Barcelona recibió 500 mil turistas en 2025.",
        &[],
        &["Barcelona"],
    ),
    (
        "es",
        "Ámsterdam redujo las emisiones un 30% en diez años.",
        &[30.0],
        &["Ámsterdam"],
    ),
    (
        "es",
        "Granada aumentó la plantilla un 18% este año.",
        &[18.0],
        &["Granada"],
    ),
    // fr
    (
        "fr",
        "Le chiffre d'affaires a augmenté de 25% à Paris.",
        &[25.0],
        &["Paris"],
    ),
    (
        "fr",
        "Les ventes ont progressé de 12,5% en Europe.",
        &[12.5],
        &["Europe"],
    ),
    (
        "fr",
        "Lyon a enregistré une baisse de 7% en 2024.",
        &[7.0],
        &["Lyon"],
    ),
    (
        "fr",
        "Le projet a atteint 2 millions d'utilisateurs.",
        &[],
        &[],
    ),
    (
        "fr",
        "Marseille organise un festival avec 40% de visiteurs étrangers.",
        &[40.0],
        &["Marseille"],
    ),
    (
        "fr",
        "Bordeaux a consacré 15% du budget à la publicité.",
        &[15.0],
        &["Bordeaux"],
    ),
    ("fr", "Nice croît de 3% chaque année.", &[3.0], &["Nice"]),
    (
        "fr",
        "Nantes a accueilli 500 mille touristes en 2025.",
        &[],
        &["Nantes"],
    ),
    (
        "fr",
        "Strasbourg a réduit ses émissions de 30% en dix ans.",
        &[30.0],
        &["Strasbourg"],
    ),
    (
        "fr",
        "Toulouse a augmenté ses effectifs de 18% cette année.",
        &[18.0],
        &["Toulouse"],
    ),
    // ru
    (
        "ru",
        "Выручка выросла на 25% в Москве.",
        &[25.0],
        &["Москве"],
    ),
    (
        "ru",
        "Продажи выросли на 12,5% в Европе.",
        &[12.5],
        &["Европе"],
    ),
    (
        "ru",
        "Казань зафиксировала падение на 7% в 2024 году.",
        &[7.0],
        &["Казань"],
    ),
    ("ru", "Проект достиг 2 миллионов пользователей.", &[], &[]),
    (
        "ru",
        "Петербург проводит фестиваль с 40% иностранных гостей.",
        &[40.0],
        &["Петербург"],
    ),
    (
        "ru",
        "Новосибирск потратил 15% бюджета на рекламу.",
        &[15.0],
        &["Новосибирск"],
    ),
    ("ru", "Сочи растёт на 3% каждый год.", &[3.0], &["Сочи"]),
    (
        "ru",
        "Самара приняла 500 тысяч туристов в 2025 году.",
        &[],
        &["Самара"],
    ),
    (
        "ru",
        "Амстердам сократил выбросы на 30% за десять лет.",
        &[30.0],
        &["Амстердам"],
    ),
    (
        "ru",
        "Минск увеличил штат на 18% в этом году.",
        &[18.0],
        &["Минск"],
    ),
    // pl
    (
        "pl",
        "Przychody wzrosły o 25% w Warszawie.",
        &[25.0],
        &["Warszawie"],
    ),
    (
        "pl",
        "Sprzedaż wzrosła o 12,5% w Europie.",
        &[12.5],
        &["Europie"],
    ),
    (
        "pl",
        "Kraków odnotował spadek o 7% w 2024 roku.",
        &[7.0],
        &["Kraków"],
    ),
    ("pl", "Projekt osiągnął 2 miliony użytkowników.", &[], &[]),
    (
        "pl",
        "Gdańsk organizuje festiwal z 40% zagranicznych gości.",
        &[40.0],
        &["Gdańsk"],
    ),
    (
        "pl",
        "Wrocław wydał 15% budżetu na reklamę.",
        &[15.0],
        &["Wrocław"],
    ),
    ("pl", "Poznań rośnie o 3% co roku.", &[3.0], &["Poznań"]),
    (
        "pl",
        "Łódź przyjęła 500 tysięcy turystów w 2025 roku.",
        &[],
        &["Łódź"],
    ),
    (
        "pl",
        "Amsterdam obniżył emisje o 30% w ciągu dziesięciu lat.",
        &[30.0],
        &["Amsterdam"],
    ),
    (
        "pl",
        "Szczecin zwiększył zatrudnienie o 18% w tym roku.",
        &[18.0],
        &["Szczecin"],
    ),
    // tr
    (
        "tr",
        "Ciro %25 arttı ve İstanbul'da zirve yaptı.",
        &[25.0],
        &["İstanbul"],
    ),
    (
        "tr",
        "Satışlar Avrupa'da %12,5 yükseldi.",
        &[12.5],
        &["Avrupa"],
    ),
    (
        "tr",
        "Ankara 2024'te %7 düşüş bildirdi.",
        &[7.0],
        &["Ankara"],
    ),
    ("tr", "Proje 2 milyon kullanıcıya ulaştı.", &[], &[]),
    (
        "tr",
        "İzmir %40 yabancı ziyaretçiyle bir festival düzenliyor.",
        &[40.0],
        &["İzmir"],
    ),
    (
        "tr",
        "Bursa reklam için bütçenin %15'ini harcadı.",
        &[15.0],
        &["Bursa"],
    ),
    ("tr", "Antalya her yıl %3 büyüyor.", &[3.0], &["Antalya"]),
    (
        "tr",
        "Konya 2025'te 500 bin turist ağırladı.",
        &[],
        &["Konya"],
    ),
    (
        "tr",
        "Amsterdam emisyonları on yılda %30 azalttı.",
        &[30.0],
        &["Amsterdam"],
    ),
    (
        "tr",
        "Adana bu yıl personeli %18 artırdı.",
        &[18.0],
        &["Adana"],
    ),
    // id
    (
        "id",
        "Pendapatan naik 25% di Jakarta.",
        &[25.0],
        &["Jakarta"],
    ),
    ("id", "Penjualan naik 12,5% di Eropa.", &[12.5], &["Eropa"]),
    (
        "id",
        "Surabaya mencatat penurunan 7% pada 2024.",
        &[7.0],
        &["Surabaya"],
    ),
    ("id", "Proyek mencapai 2 juta pengguna.", &[], &[]),
    (
        "id",
        "Bandung menggelar festival dengan 40% pengunjung asing.",
        &[40.0],
        &["Bandung"],
    ),
    (
        "id",
        "Medan menghabiskan 15% anggaran untuk iklan.",
        &[15.0],
        &["Medan"],
    ),
    ("id", "Bali tumbuh 3% setiap tahun.", &[3.0], &["Bali"]),
    (
        "id",
        "Makassar menerima 500 ribu wisatawan pada 2025.",
        &[],
        &["Makassar"],
    ),
    (
        "id",
        "Amsterdam memangkas emisi 30% dalam sepuluh tahun.",
        &[30.0],
        &["Amsterdam"],
    ),
    (
        "id",
        "Yogyakarta menambah karyawan 18% tahun ini.",
        &[18.0],
        &["Yogyakarta"],
    ),
];

/// Directory holding the UDPipe models: `VISUALNER_MODEL_DIR`, or the
/// binary's default location under `$HOME`.
fn model_dir() -> PathBuf {
    match std::env::var_os("VISUALNER_MODEL_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
            .join(".local/share/visualner/models"),
    }
}

/// Runs every case through one binary process with the given model directory.
/// Each request is one NDJSON line and each response is one line, in order.
fn run_suite(dir: &Path) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_visualner"))
        .env("VISUALNER_MODEL_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn visualner");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for (language, text, _, _) in CASES {
            let request = json!({
                "version": "visualner.v1",
                "operation": "analyze",
                "source_text": text,
                "language": language,
                "entity_count": 5,
            });
            writeln!(stdin, "{request}").expect("write request");
        }
    }
    let stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let responses: Vec<Value> = stdout
        .lines()
        .take(CASES.len())
        .map(|line| serde_json::from_str(&line.expect("read response")).expect("valid JSON"))
        .collect();
    let status = child.wait().expect("wait visualner");
    assert!(status.success(), "visualner exited with {status}");
    assert_eq!(responses.len(), CASES.len(), "one response per request");
    responses
}

/// Checks every response against the fixed expectations and returns the list
/// of failures, so one run reports all problems instead of the first one.
fn check(mode: &str, responses: &[Value], require_proper_nouns: bool) -> Vec<String> {
    let mut failures = Vec::new();
    for ((language, text, charts, proper), response) in CASES.iter().zip(responses) {
        if !response["error"].is_null() {
            failures.push(format!(
                "[{mode}][{language}] {text:?}: error {}",
                response["error"]
            ));
            continue;
        }
        let values: Vec<f64> = response["chart_points"]
            .as_array()
            .map(|points| points.iter().filter_map(|p| p["value"].as_f64()).collect())
            .unwrap_or_default();
        if values != *charts {
            failures.push(format!(
                "[{mode}][{language}] {text:?}: chart values {values:?}, expected {charts:?}"
            ));
        }
        if require_proper_nouns {
            let entities: Vec<&str> = response["entities"]
                .as_array()
                .map(|list| list.iter().filter_map(|e| e["text"].as_str()).collect())
                .unwrap_or_default();
            for noun in *proper {
                if !entities.iter().any(|entity| entity.contains(noun)) {
                    failures.push(format!(
                        "[{mode}][{language}] {text:?}: missing entity {noun:?} in {entities:?}"
                    ));
                }
            }
        }
    }
    failures
}

#[test]
fn suite_covers_ten_sentences_per_language() {
    let languages = ["it", "en", "de", "es", "fr", "ru", "pl", "tr", "id"];
    for language in languages {
        let count = CASES.iter().filter(|case| case.0 == language).count();
        assert_eq!(count, 10, "{language} must have 10 sentences");
    }
    assert_eq!(CASES.len(), 90);
}

#[test]
fn model_mode_matches_expected_entities_and_numbers() {
    let dir = model_dir();
    // Fail loudly when a model is missing: a silent skip would hide a broken
    // regression suite.
    for file in [
        "english-ewt",
        "italian-isdt",
        "polish-pdb",
        "russian-syntagrus",
        "german-gsd",
        "spanish-gsd",
        "french-gsd",
        "turkish-imst",
        "indonesian-gsd",
    ] {
        let found = std::fs::read_dir(&dir)
            .expect("model directory exists")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(file));
        assert!(found, "missing UDPipe model {file}* in {}", dir.display());
    }
    let responses = run_suite(&dir);
    let failures = check("model", &responses, true);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn fallback_mode_matches_expected_numbers() {
    let dir = std::env::temp_dir().join("visualner-empty-models");
    std::fs::create_dir_all(&dir).expect("create empty model dir");
    let responses = run_suite(&dir);
    let failures = check("fallback", &responses, false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
