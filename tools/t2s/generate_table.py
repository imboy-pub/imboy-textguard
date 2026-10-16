#!/usr/bin/env python3
"""Generate the restricted single-scalar t2s table from locked OpenCC data.

Filters OpenCC TSCharacters to unambiguous single-char -> single-char
mappings, resolves chains to a fixed point, rejects cycles, and proves the
final table idempotent (plan 4.4). Runs offline against a pinned input file;
the committed output is the build-time artifact embedded by the binary.

Usage:
    generate_table.py --input TSCharacters.txt --out-dir data/t2s
    generate_table.py --selftest <tmpdir>   # run twice, compare digests
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

GENERATOR_VERSION = "1"
TABLE_ID = "opencc-1.1.9-single-v1"
SOURCE_URL = ("https://raw.githubusercontent.com/BYVoid/OpenCC/ver.1.1.9/"
              "data/dictionary/TSCharacters.txt")


def parse_entries(text):
    entries = {}
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise ValueError(f"malformed dictionary line: {line!r}")
        traditional, targets_field = parts
        targets = targets_field.split(" ")
        if traditional in entries and entries[traditional] != targets:
            raise ValueError(f"conflicting duplicate key: {traditional!r}")
        entries[traditional] = targets
    return entries


def build(entries):
    for key, targets in entries.items():
        if len(key) != 1 or any(len(target) != 1 for target in targets):
            raise ValueError(f"multi-scalar key or target is forbidden: {key!r}")
    unique = {key: targets[0] for key, targets in entries.items()
              if len(targets) == 1 and targets[0] != key}
    excluded_multi = sorted(key for key, targets in entries.items()
                            if len(targets) != 1)
    excluded_identity = sorted(key for key, targets in entries.items()
                               if len(targets) == 1 and targets[0] == key)
    table = {}
    excluded_cycles = []
    for key in sorted(unique):
        current = unique[key]
        seen = {key}
        while current in unique:
            if current in seen:
                break
            seen.add(current)
            current = unique[current]
        else:
            table[key] = current
            continue
        # walk hit a cycle that does not return to a fixed point
        excluded_cycles.append(key)
    # exhaustive idempotence proof: T(T(x)) == T(x) for every mapped scalar
    for key, value in table.items():
        if table.get(value, value) != value:
            raise AssertionError(f"non-idempotent mapping for {key!r}")
    return table, excluded_multi, excluded_identity, excluded_cycles


def render(table):
    lines = []
    for key in sorted(table, key=ord):
        lines.append(f"{key}\t{table[key]}\n")
    return "".join(lines)


def build_from_input(input_path):
    raw = Path(input_path).read_bytes()
    entries = parse_entries(raw.decode("utf-8"))
    table, multi, identity, cycles = build(entries)
    table_text = render(table)
    report = {
        "table_id": TABLE_ID,
        "generator_version": GENERATOR_VERSION,
        "source_url": SOURCE_URL,
        "source_sha256": hashlib.sha256(raw).hexdigest(),
        "input_entries": len(entries),
        "table_size": len(table),
        "excluded_multi_target": len(multi),
        "excluded_identity": len(identity),
        "excluded_cycle": len(cycles),
        "exclusions_sha256": hashlib.sha256(
            "\n".join(multi + identity + cycles).encode("utf-8")).hexdigest(),
        "table_sha256": hashlib.sha256(table_text.encode("utf-8")).hexdigest(),
        "idempotence": "T(T(x)) == T(x) verified exhaustively",
    }
    exclusions_text = "".join(
        f"multi\t{key}\n" for key in multi
    ) + "".join(f"identity\t{key}\n" for key in identity
                ) + "".join(f"cycle\t{key}\n" for key in cycles)
    return table_text, report, exclusions_text


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input")
    parser.add_argument("--out-dir")
    parser.add_argument("--selftest", metavar="TMPDIR",
                        help="generate twice into the directory and compare")
    args = parser.parse_args()
    if args.selftest and not args.input:
        parser.error("--selftest also requires --input")
    if args.selftest:
        base = Path(args.selftest)
        base.mkdir(parents=True, exist_ok=True)
        digests = []
        for round_index in (1, 2):
            table_text, report, _ = build_from_input(args.input)
            destination = base / f"round{round_index}"
            destination.mkdir()
            (destination / "single_table.tsv").write_text(table_text, encoding="utf-8")
            digests.append(hashlib.sha256(table_text.encode("utf-8")).hexdigest())
        ok = digests[0] == digests[1]
        print(json.dumps({"reproducible": ok, "digests": digests}))
        return 0 if ok else 1
    if not args.input or not args.out_dir:
        parser.error("--input and --out-dir are required (or --selftest)")
    table_text, report, exclusions_text = build_from_input(args.input)
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    (out / "single_table.tsv").write_text(table_text, encoding="utf-8")
    (out / "excluded.tsv").write_text(exclusions_text, encoding="utf-8")
    (out / "table_report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8")
    print(json.dumps({key: report[key] for key in
                      ("table_size", "excluded_multi_target", "excluded_cycle",
                       "table_sha256")}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
