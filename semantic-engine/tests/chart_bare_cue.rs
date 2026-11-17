//! Bare Polish/Russian conjunction as a chart cue, with a precision gate.

mod common;

use common::*;
use serde_json::Value;

fn charts(value: &Value) -> Vec<&Value> {
    proposals_of(&value["scenes"][0], "chart")
}

#[test]
fn polish_bare_a_binds_two_grounded_percentages() {
    let text = "Firma Woda Nord oszacowała oszczędność 28%, a Energia Port przewidywała 13%.";
    let value = manifest_value_for("pl", vec![scene("pl-bare-a", text)]);
    let charts = charts(&value);
    assert_eq!(charts.len(), 1, "one chart expected: {value}");
    let values: Vec<f64> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    assert_eq!(values, [28.0, 13.0]);
}

#[test]
fn russian_bare_a_binds_two_grounded_percentages() {
    for (id, text, expected) in [
        (
            "ru-bare-a-1",
            "Компания Вода Север оценила экономию в 26%, а Энергия Порт предполагала 11%.",
            [26.0, 11.0],
        ),
        (
            "ru-bare-a-2",
            "Бюро Волга ожидало экономию в 25%, а бюро Заря заявляло 41%.",
            [25.0, 41.0],
        ),
    ] {
        let value = manifest_value_for("ru", vec![scene(id, text)]);
        let charts = charts(&value);
        assert_eq!(charts.len(), 1, "one chart expected for {id}: {value}");
        let values: Vec<f64> = charts[0]["chart_items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["value"].as_f64().unwrap())
            .collect();
        assert_eq!(values, expected);
    }
}

#[test]
fn twenty_ordinary_conjunction_paragraphs_emit_no_chart() {
    let cases = [
        ("pl", "Anna poszła do sklepu a potem wróciła do domu."),
        (
            "pl",
            "W 2019 roku zespół zagrał koncert a publiczność wypełniła halę.",
        ),
        (
            "pl",
            "Firma zatrudniła 12 osób a biuro przeniosła do Gdańska.",
        ),
        ("pl", "Kowalska miała 19 lat a jej brat uczył się w liceum."),
        (
            "pl",
            "Projekt kosztował 40 tysięcy euro a budowa trwała dwa lata.",
        ),
        (
            "pl",
            "Deszcz padał przez trzy godziny a wiatr uszkodził dach.",
        ),
        (
            "pl",
            "W 2003 roku zapłaciła 12 euro za bilet a zwiedzanie trwało godzinę.",
        ),
        (
            "pl",
            "Biuro otwarto o 8 rano a klienci czekali na zewnątrz.",
        ),
        (
            "pl",
            "Rada przyjęła 27 głosów a opozycja zgłosiła poprawkę.",
        ),
        ("pl", "Stadion ma 18 tysięcy miejsc a murawa jest nowa."),
        ("ru", "Анна пошла в магазин, а потом вернулась домой."),
        ("ru", "В 2019 году команда дала концерт, а зал был полон."),
        (
            "ru",
            "Компания наняла 12 человек, а офис перенесла в Казань.",
        ),
        ("ru", "Кузнецовой было 19 лет, а её брат учился в школе."),
        (
            "ru",
            "Проект стоил 40 тысяч евро, а стройка длилась два года.",
        ),
        ("ru", "Дождь шёл три часа, а ветер повредил крышу."),
        (
            "ru",
            "В 2003 году она заплатила 12 евро за билет, а экскурсия длилась час.",
        ),
        ("ru", "Офис открылся в 8 утра, а клиенты ждали на улице."),
        (
            "ru",
            "Совет принял 27 голосов, а оппозиция внесла поправку.",
        ),
        ("ru", "Стадион вмещает 18 тысяч мест, а поле новое."),
    ];
    for (index, (language, text)) in cases.iter().enumerate() {
        let value =
            manifest_value_for(language, vec![scene(&format!("negative-{index:02}"), text)]);
        assert!(
            charts(&value).is_empty(),
            "ordinary conjunction produced a chart in {language}: {text}\n{value}"
        );
    }
}

#[test]
fn italian_a_is_never_a_contrastive_cue() {
    let text = "La percentuale è passata dal 28% al 13% senza una spiegazione.";
    let value = manifest_value_for("it", vec![scene("it-preposition", text)]);
    assert!(
        charts(&value).is_empty(),
        "Italian a/al is a preposition, not a chart cue: {value}"
    );
}
