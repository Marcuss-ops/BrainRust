#!/usr/bin/env python3
"""Score a draft, dimension-complete quantity matrix against the engine.

The cases are deliberately minimal: ten grounded measures for every unit
dimension and 20 negative numeric forms in each supported language, plus scale,
qualifier and period examples. The annotations are assistant drafts and do not
certify natural-language recall.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
UNITS = json.loads((ROOT / "lexicon/units.json").read_text(encoding="utf-8"))
LANGUAGES = ("de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr")
MEASURE_CONTEXTS = {
    "de": ("Im Bericht beträgt der Messwert {measure}.", "Die Tabelle nennt {measure} für diesen Zeitraum.", "Nach den vorliegenden Daten liegt der Wert bei {measure}."),
    "en": ("The report puts the measured value at {measure}.", "The table lists {measure} for this period.", "According to the available data, the value is {measure}."),
    "es": ("El informe sitúa la medida en {measure}.", "La tabla recoge {measure} para este periodo.", "Según los datos disponibles, el valor es de {measure}."),
    "fr": ("Le rapport situe la mesure à {measure}.", "Le tableau indique {measure} pour cette période.", "D'après les données disponibles, la valeur est de {measure}."),
    "id": ("Laporan itu mencatat ukuran sebesar {measure}.", "Tabel tersebut memuat {measure} untuk periode ini.", "Menurut data yang tersedia, nilainya adalah {measure}."),
    "it": ("Nel rapporto la misura è pari a {measure}.", "La tabella riporta {measure} per questo periodo.", "Secondo i dati disponibili, il valore è di {measure}."),
    "nl": ("Het rapport stelt de meting vast op {measure}.", "De tabel vermeldt {measure} voor deze periode.", "Volgens de beschikbare gegevens bedraagt de waarde {measure}."),
    "pl": ("Raport podaje wartość pomiaru: {measure}.", "Tabela zawiera {measure} dla tego okresu.", "Według dostępnych danych wartość wynosi {measure}."),
    "pt": ("O relatório situa a medida em {measure}.", "A tabela regista {measure} para este período.", "Segundo os dados disponíveis, o valor é de {measure}."),
    "ru": ("В отчёте указано значение измерения: {measure}.", "В таблице приведено {measure} за этот период.", "Согласно имеющимся данным, значение составляет {measure}."),
    "tr": ("Raporda ölçüm değeri {measure} olarak veriliyor.", "Tabloda bu dönem için {measure} yer alıyor.", "Mevcut verilere göre değer {measure} düzeyinde."),
}
NEGATIVES = (
    "2019", "123456789", "1st", "3 times", "3 volte", "APT 3",
    "123 Main St", "ID-42", "Phone 555-0142", "Room 12", "Version 2.3.8",
    "Line 4", "Page 27", "No. 56", "Route 66", "Section 9", "Seat 14",
    "3x", "1:3", "2024-05-03",
)


def cases(language: str) -> list[dict]:
    rows = []
    language_units = UNITS["languages"][language]["units"]
    for dimension in UNITS["dimensions"]:
        unit = next(row for row in language_units if row["dimension"] == dimension)
        if dimension == "currency":
            unit = next(row for row in language_units if row["code"] == "EUR")
            surface = "EUR"
        elif dimension == "percent":
            surface = "%"
        elif dimension == "length":
            unit = next(row for row in language_units if row["code"] == "km")
            surface = unit["surfaces"][0]
        elif dimension == "speed":
            surface = next(surface for surface in unit["surfaces"] if "/" in surface)
        elif dimension == "temperature":
            surface = next((surface for surface in unit["surfaces"] if surface.startswith("°")), unit["surfaces"][0])
        else:
            surface = unit["surfaces"][0]
        code = unit["code"]
        factor = unit["factor_to_base"]
        for value in range(1, 11):
            measure = f"{value}%" if dimension == "percent" else (
                f"{value}{surface}" if dimension == "temperature" and surface.startswith("°")
                else f"{value} {surface}"
            )
            text = MEASURE_CONTEXTS[language][(value - 1) % len(MEASURE_CONTEXTS[language])].format(measure=measure)
            rows.append({
                "scene_id": f"{language}_{dimension}_{value:02}",
                "text": text,
                "expected": [{
                    "evidence": str(value) + ("%" if dimension == "percent" else ""),
                    "dimension": dimension,
                    "unit": code,
                    "value": value * factor,
                }],
                "negative": False,
            })
    scale_surface = {
        "de": "million", "en": "million", "es": "millón", "fr": "million",
        "id": "juta", "it": "milioni", "nl": "miljoen", "pl": "milionów",
        "pt": "milhão", "ru": "миллионов", "tr": "milyon",
    }[language]
    rows.append({
        "scene_id": f"{language}_scaled_currency_01",
        "text": f"3 {scale_surface} EUR",
        "expected": [{"evidence": f"3 {scale_surface}", "dimension": "currency",
                      "unit": "EUR", "value": 3_000_000.0,
                      "scale": 1_000_000.0}],
        "negative": False,
    })
    approximate_cue = UNITS["languages"][language]["qualifiers"]["approx"][0]
    rows.extend((
        {
            "scene_id": f"{language}_approx_01",
            "text": f"{approximate_cue} 5%",
            "expected": [{"evidence": "5%", "dimension": "percent", "unit": "PCT",
                          "value": 5.0, "qualifier": "approx"}],
            "negative": False,
        },
        {
            "scene_id": f"{language}_period_01",
            "text": "5 EUR in 2019",
            "expected": [{"evidence": "5", "dimension": "currency", "unit": "EUR",
                          "value": 5.0, "qualifier": "exact", "period_year": 2019}],
            "negative": False,
        },
    ))
    if language == "it":
        rows.extend((
            {
                "scene_id": "it_ratio_01",
                "text": "Il valore è 10% rispetto al doppio di 5%.",
                "expected": [
                    {"evidence": "10%", "dimension": "percent", "unit": "PCT", "value": 10.0},
                    {"evidence": "5%", "dimension": "percent", "unit": "PCT", "value": 5.0},
                ],
                "expected_comparisons": [{"kind": "ratio", "dimension": "percent", "ratio": 2.0}],
                "negative": False,
            },
            {
                "scene_id": "it_cross_currency_01",
                "text": "Il valore è 10 EUR contro 5 USD.",
                "expected": [
                    {"evidence": "10", "dimension": "currency", "unit": "EUR", "value": 10.0},
                    {"evidence": "5", "dimension": "currency", "unit": "USD", "value": 5.0},
                ],
                "expected_comparisons": [], "negative": False,
            },
            {
                "scene_id": "it_cross_dimension_01",
                "text": "Il gruppo ha 42% contro 18 euro.",
                "expected": [
                    {"evidence": "42%", "dimension": "percent", "unit": "PCT", "value": 42.0},
                    {"evidence": "18", "dimension": "currency", "unit": "EUR", "value": 18.0},
                ],
                "expected_comparisons": [], "negative": False,
            },
        ))
    for index, text in enumerate(NEGATIVES, 1):
        rows.append({
            "scene_id": f"{language}_negative_{index:02}",
            "text": text,
            "expected": [],
            "negative": True,
        })
    for row in rows:
        row.setdefault("expected_comparisons", [])
    return rows


def run_worker(worker: Path, language: str, rows: list[dict]) -> list[dict]:
    request = {
        "operation": "analyze_script",
        "language": language,
        "scenes": [
            {"scene_id": row["scene_id"], "text": row["text"]}
            for row in rows
        ],
    }
    result = subprocess.run(
        [str(worker)], input=json.dumps(request, ensure_ascii=False) + "\n",
        text=True, capture_output=True, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"worker exited {result.returncode}: {result.stderr}")
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if len(lines) != 1:
        raise RuntimeError(f"worker returned {len(lines)} lines for {language}")
    response = json.loads(lines[0])
    if not response.get("ok"):
        raise RuntimeError(f"worker rejected {language}: {response}")
    return response["manifest"]["scenes"]


def score_language(rows: list[dict], scenes: list[dict]) -> dict:
    expected_total = extracted_total = matched = false_positives = negative_fp = 0
    exact_value = exact_scale = exact_unit = grounded = qualifier_expected = qualifier_matched = 0
    period_expected = period_matched = period_predictions = 0
    comparison_expected = comparison_matched = comparison_predictions = comparison_false_positives = 0
    by_dimension: dict[str, dict] = {}
    details = []
    for row, scene in zip(rows, scenes, strict=True):
        quantities = scene.get("quantities", [])
        comparisons = scene.get("quantity_comparisons", [])
        expected_comparisons = row.get("expected_comparisons", [])
        comparison_expected += len(expected_comparisons)
        comparison_predictions += len(comparisons)
        unused_comparisons = list(comparisons)
        for gold in expected_comparisons:
            hit = next((index for index, comparison in enumerate(unused_comparisons)
                        if comparison.get("kind") == gold["kind"]
                        and comparison.get("dimension") == gold["dimension"]
                        and ("ratio" not in gold or math.isclose(comparison.get("ratio", math.nan), gold["ratio"], rel_tol=0.0, abs_tol=1e-9))
                        and ("difference" not in gold or math.isclose(comparison.get("difference", math.nan), gold["difference"], rel_tol=0.0, abs_tol=1e-9))), None)
            if hit is None:
                details.append({"scene_id": row["scene_id"], "missing_comparison": gold, "actual": comparisons})
            else:
                unused_comparisons.pop(hit)
                comparison_matched += 1
        comparison_false_positives += len(unused_comparisons)
        if unused_comparisons:
            details.append({"scene_id": row["scene_id"], "false_positive_comparisons": unused_comparisons})
        extracted_total += len(quantities)
        period_predictions += sum(
            (quantity.get("period") or {}).get("year") is not None
            for quantity in quantities
        )
        if row["negative"]:
            negative_fp += len(quantities)
            if quantities:
                details.append({"scene_id": row["scene_id"], "false_positives": quantities})
            continue
        expected = row["expected"]
        expected_total += len(expected)
        unused = list(quantities)
        for gold in expected:
            dimension = gold["dimension"]
            bucket = by_dimension.setdefault(dimension, {"expected": 0, "matched": 0, "exact_values": 0})
            bucket["expected"] += 1
            hit = next((index for index, quantity in enumerate(unused)
                        if quantity.get("evidence") == gold["evidence"]
                        and quantity.get("dimension") == gold["dimension"]
                        and quantity.get("unit") == gold["unit"]), None)
            if hit is None:
                details.append({"scene_id": row["scene_id"], "miss": gold, "actual": quantities})
                continue
            quantity = unused.pop(hit)
            matched += 1
            exact_unit += 1
            bucket["matched"] += 1
            if gold.get("qualifier", "exact") != "exact":
                qualifier_expected += 1
                qualifier_matched += quantity.get("qualifier") == gold["qualifier"]
            actual_period = (quantity.get("period") or {}).get("year")
            if gold.get("period_year") is not None:
                period_expected += 1
                period_matched += actual_period == gold["period_year"]
            if actual_period is not None:
                if actual_period != gold.get("period_year"):
                    details.append({"scene_id": row["scene_id"], "unexpected_period": quantity.get("period")})
            value = quantity.get("value")
            if isinstance(value, (int, float)) and math.isclose(value, gold["value"], rel_tol=0.0, abs_tol=1e-9):
                exact_value += 1
                bucket["exact_values"] += 1
            exact_scale += quantity.get("scale") == gold.get("scale")
            start, end = quantity.get("start_byte"), quantity.get("end_byte")
            raw = row["text"].encode("utf-8")
            if isinstance(start, int) and isinstance(end, int) and raw[start:end].decode("utf-8", errors="replace") == quantity["evidence"]:
                grounded += 1
            else:
                details.append({"scene_id": row["scene_id"], "bad_span": quantity})
        if unused:
            false_positives += len(unused)
            details.append({"scene_id": row["scene_id"], "false_positives": unused})

    precision = matched / extracted_total if extracted_total else 1.0
    recall = matched / expected_total if expected_total else 1.0
    for bucket in by_dimension.values():
        expected = bucket["expected"]
        bucket["recall"] = bucket["matched"] / expected if expected else 1.0
        bucket["exact_value_rate"] = bucket["exact_values"] / bucket["matched"] if bucket["matched"] else 1.0
    return {
        "cases": len(rows), "positive_expected": expected_total,
        "extracted": extracted_total, "matched": matched,
        "precision": precision, "recall": recall,
        "exact_value_rate": exact_value / extracted_total if extracted_total else 1.0,
        "exact_scale_rate": exact_scale / extracted_total if extracted_total else 1.0,
        "unit_recall": exact_unit / expected_total if expected_total else 1.0,
        "grounded_rate": grounded / extracted_total if extracted_total else 1.0,
        "qualifier_expected": qualifier_expected,
        "qualifier_recall": qualifier_matched / qualifier_expected if qualifier_expected else 1.0,
        "period_expected": period_expected,
        "period_binding_precision": period_matched / period_predictions if period_predictions else (1.0 if not period_expected else 0.0),
        "period_binding_recall": period_matched / period_expected if period_expected else 1.0,
        "comparisons_expected": comparison_expected,
        "comparisons_matched": comparison_matched,
        "comparison_precision": comparison_matched / comparison_predictions if comparison_predictions else (1.0 if not comparison_expected else 0.0),
        "comparison_recall": comparison_matched / comparison_expected if comparison_expected else 1.0,
        "comparison_false_positives": comparison_false_positives,
        "by_dimension": by_dimension,
        "negative_false_positives": negative_fp,
        "positive_false_positives": false_positives,
        "failures": details,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=ROOT / "target/release/semantic-engine")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if not args.worker.is_file():
        parser.error(f"worker not found: {args.worker}; build semantic-engine first")
    report = {
        "schema": "brainrust.quantity_gold_report.v1",
        "annotation_authority": "assistant_draft_pending_human_review",
        "worker_sha256": hashlib.sha256(args.worker.read_bytes()).hexdigest(),
        "languages": {},
    }
    for language in LANGUAGES:
        rows = cases(language)
        report["languages"][language] = score_language(
            rows, run_worker(args.worker, language, rows)
        )
    report["totals"] = {
        key: sum(language[key] for language in report["languages"].values())
        for key in ("cases", "positive_expected", "extracted", "matched",
                    "negative_false_positives", "positive_false_positives")
    }
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    passed = all(
        row["exact_value_rate"] == 1.0
        and row["exact_scale_rate"] == 1.0
        and row["unit_recall"] >= 0.9
        and row["grounded_rate"] == 1.0
        and row["qualifier_recall"] >= 0.9
        and row["period_binding_precision"] >= 0.95
        and row["period_binding_recall"] >= 0.95
        and row["comparison_precision"] == 1.0
        and row["comparison_recall"] >= 0.9
        and row["comparison_false_positives"] == 0
        and row["negative_false_positives"] == 0
        for row in report["languages"].values()
    )
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
