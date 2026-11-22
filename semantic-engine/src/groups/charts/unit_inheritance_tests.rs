use super::inherit_explicit_units;
use crate::chart_rules::ChartUnit;

fn measure(kind: &str, signature: &str) -> ChartUnit {
    ChartUnit {
        kind: kind.to_string(),
        signature: signature.to_string(),
        value: 1.0,
    }
}

#[test]
fn bare_measures_inherit_the_single_explicit_unit() {
    let mut measures = vec![measure("cars", "120 cars"), measure("cars", "cars")];
    inherit_explicit_units(&mut measures);
    assert_eq!(measures[1].signature, "120 cars");
    assert_eq!(measures[1].kind, "cars");
}

#[test]
fn conflicting_explicit_units_leave_the_series_unchanged() {
    let mut measures = vec![
        measure("EUR", "€ 120"),
        measure("USD", "$ 180"),
        measure("EUR", "EUR"),
    ];
    inherit_explicit_units(&mut measures);
    assert_eq!(measures[0].signature, "€ 120");
    assert_eq!(measures[1].signature, "$ 180");
    assert_eq!(measures[2].signature, "EUR");
}
