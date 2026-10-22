#!/usr/bin/env python3
"""Regenerate ``lexicon/months.json`` from the system ICU (CLDR month names).

Month names are no longer hand-typed: this tool extracts the CLDR wide month
names (format + standalone contexts) for the pipeline's date-scan languages
straight from the ICU the workspace links against, so the table can never
drift from the locale data behind the runtime.

The generated file keeps the extraction behaviour of the historical hand
table through two explicit, documented policy layers:

* ``date_scan_exclusions`` — names dropped only from date-span scanning
  (English "may" is a modal verb, not evidence of a date);
* ``legacy_aliases`` — spellings the pipeline matched historically that CLDR
  does not emit (accent-stripped French forms).

Usage:
    python3 sync_cldr_months.py          # write lexicon/months.json
    python3 sync_cldr_months.py --check  # fail if the file is stale
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "months.json"

# Date-scan languages: the locales whose month names the pipeline has always
# recognized in text. Turkish and Indonesian have CLDR month names too, but no
# month-name matching today; enabling them is a deliberate coverage decision,
# not a data-sync side effect.
LOCALES = ["en", "it", "es", "fr", "de", "pt", "pl", "ru"]

# Names excluded from the date-span scan only. The entity-typing view matches
# whole tokens and keeps them.
DATE_SCAN_EXCLUSIONS = {
    "may": "English modal verb 'may' is not evidence of a date; kept for "
    "whole-token entity typing only",
}

# Spellings matched historically that CLDR does not emit. Kept so the
# data-driven table is a superset of the certified hand table.
LEGACY_ALIASES = {
    "aout": (8, "historical accent-stripped French spelling of 'aout'"),
    "decembre": (12, "historical accent-stripped French spelling of 'decembre'"),
}

PROBE_C = r"""
#include <stdio.h>
#include <stdlib.h>
#include <unicode/udat.h>
#include <unicode/ustring.h>
#include <unicode/uversion.h>

static void print_symbols(const UDateFormat *fmt, UDateFormatSymbolType type,
                          const char *key) {
    int32_t count = udat_countSymbols(fmt, type);
    printf("  \"%s\": [", key);
    for (int32_t i = 0; i < count; i++) {
        UChar buf[128];
        char utf8[512];
        UErrorCode status = U_ZERO_ERROR;
        int32_t len = udat_getSymbols(fmt, type, i, buf, 128, &status);
        if (U_FAILURE(status)) {
            fprintf(stderr, "udat_getSymbols(%s, %d): %s\n", key, i,
                    u_errorName(status));
            exit(2);
        }
        int32_t utf8_len = 0;
        u_strToUTF8(utf8, (int32_t)sizeof utf8, &utf8_len, buf, len, &status);
        if (U_FAILURE(status)) {
            fprintf(stderr, "u_strToUTF8: %s\n", u_errorName(status));
            exit(2);
        }
        printf("%s\"%s\"", i ? ", " : "", utf8);
    }
    printf("]");
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: probe <locale>\n");
        return 2;
    }
    UErrorCode status = U_ZERO_ERROR;
    UDateFormat *fmt =
        udat_open(UDAT_DEFAULT, UDAT_DEFAULT, argv[1], NULL, -1, NULL, 0, &status);
    if (U_FAILURE(status)) {
        fprintf(stderr, "udat_open(%s): %s\n", argv[1], u_errorName(status));
        return 2;
    }
    UVersionInfo version;
    char version_string[U_MAX_VERSION_STRING_LENGTH];
    u_getVersion(version);
    u_versionToString(version, version_string);

    printf("{\n  \"icu_version\": \"%s\",\n  \"locale\": \"%s\",\n", version_string,
           argv[1]);
    print_symbols(fmt, UDAT_MONTHS, "format");
    printf(",\n");
    print_symbols(fmt, UDAT_STANDALONE_MONTHS, "standalone");
    printf("\n}\n");
    udat_close(fmt);
    return 0;
}
"""


def build_probe() -> Path:
    cflags = subprocess.run(
        ["pkg-config", "--cflags", "icu-i18n", "icu-uc"],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    libs = subprocess.run(
        ["pkg-config", "--libs", "icu-i18n", "icu-uc"],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    tmp = Path(tempfile.mkdtemp(prefix="cldr-months-"))
    source = tmp / "probe.c"
    binary = tmp / "probe"
    source.write_text(PROBE_C)
    subprocess.run(
        ["cc", "-O1", "-o", str(binary), str(source), *cflags, *libs], check=True
    )
    return binary


def collect(probe: Path) -> tuple[str, dict]:
    """Returns (icu_version, {name: {"month": int, "locales": set, "contexts": set}})."""
    icu_version = None
    entries: dict = {}
    for locale in LOCALES:
        data = json.loads(
            subprocess.run([str(probe), locale], capture_output=True, text=True,
                           check=True).stdout
        )
        icu_version = data["icu_version"]
        for context in ("format", "standalone"):
            for index, raw in enumerate(data[context]):
                name = raw.strip().lower()
                if not name:
                    continue
                month = index + 1
                entry = entries.setdefault(
                    name, {"month": month, "locales": set(), "contexts": set()}
                )
                if entry["month"] != month:
                    raise SystemExit(
                        f"CLDR conflict: {name!r} is month {entry['month']} and "
                        f"{month} ({locale}, {context})"
                    )
                entry["locales"].add(locale)
                entry["contexts"].add(context)
    return icu_version, entries


def generate() -> str:
    probe = build_probe()
    icu_version, entries = collect(probe)
    for name, (month, _reason) in LEGACY_ALIASES.items():
        entries.setdefault(
            name, {"month": month, "locales": set(), "contexts": set()}
        )
    for name in DATE_SCAN_EXCLUSIONS:
        if name not in entries:
            raise SystemExit(f"exclusion {name!r} not produced by CLDR; drop it")

    months = [
        {
            "name": name,
            "month": data["month"],
            "locales": sorted(data["locales"]),
            "contexts": sorted(data["contexts"]),
        }
        for name, data in sorted(entries.items(), key=lambda kv: (kv[1]["month"], kv[0]))
    ]
    document = {
        "schema": "months.v1",
        "provenance": {
            "generator": "lexicon/tools/sync_cldr_months.py",
            "source": "system ICU (CLDR wide month names, format + standalone contexts)",
            "icu_version": icu_version,
            "locales": LOCALES,
        },
        "date_scan_exclusions": [
            {"name": name, "reason": reason}
            for name, reason in sorted(DATE_SCAN_EXCLUSIONS.items())
        ],
        "legacy_aliases": [
            {"name": name, "month": month, "reason": reason}
            for name, (month, reason) in sorted(LEGACY_ALIASES.items())
        ],
        "months": months,
    }
    return json.dumps(document, ensure_ascii=False, indent=1) + "\n"


def main() -> int:
    text = generate()
    if "--check" in sys.argv:
        if not OUT.exists() or OUT.read_text() != text:
            print(f"STALE: {OUT} — run sync_cldr_months.py", file=sys.stderr)
            return 1
        print(f"up to date: {OUT.name} ({len(json.loads(text)['months'])} names)")
        return 0
    OUT.write_text(text)
    months = json.loads(text)["months"]
    print(f"wrote {OUT} — {len(months)} month names, "
          f"{sum(1 for m in months if m['locales'])} from CLDR, "
          f"{len(LEGACY_ALIASES)} legacy aliases")
    return 0


if __name__ == "__main__":
    sys.exit(main())
