#!/usr/bin/env python3
"""Offline validation of checked-in words, rulesets, provenance and round counts."""
from __future__ import annotations
import collections
import json
from pathlib import Path
import re
import sys
from build_review_lexicon import CATEGORIES, DATA, ROOT, constraint, norm, make_pack


def validate(root: Path = DATA) -> dict:
    counts = json.loads((root / 'COUNTS.json').read_text())
    terms = [json.loads(line) for line in (root / 'provenance/terms.jsonl').read_text().splitlines()]
    assert terms and len(terms) == counts['total']
    assert len({t['id'] for t in terms}) == len(terms)
    assert len({norm(t['word']) for t in terms}) == len(terms)
    source_by_id = {s['id']: s for s in json.loads((root / 'provenance/sources.json').read_text())}
    for t in terms:
        assert t['category'] in CATEGORIES
        assert 0 < int(t['id']) < 2**63
        assert constraint(t['word'], baseline=t['first_round'] == 0) is None
        assert t['normalized'] == norm(t['word'])
        assert t['synthetic_expansion'] is False
        assert t['business_validation'] == 'NOT_VALIDATED'
        assert t['sources']
        for source in t['sources']:
            locked = source_by_id[source['source_id']]
            assert locked['blob_verified'] is True and locked['coverage'] == 'complete'
            assert re.fullmatch('[a-f0-9]{40}', locked['ref'])
            assert re.fullmatch('[a-f0-9]{40}', locked['blob'])
            assert re.fullmatch('[a-f0-9]{64}', locked['sha256'])
            assert source['line'] > 0 and norm(source['word']) == t['normalized']
    assert dict(collections.Counter(t['category'] for t in terms)) == counts['counts']
    assert counts['generated_terms'] == 0
    assert [r['round'] for r in counts['rounds']] == [1, 2, 3]
    for rnd in counts['rounds']:
        actual = collections.Counter(t['category'] for t in terms if t['first_round'] == rnd['round'])
        assert {c: actual[c] for c in CATEGORIES} == rnd['new_unique']
        assert sum(actual.values()) > 0, 'An empty collection attempt is not an expansion round'
    assert counts['baseline_retained'] + sum(sum(r['new_unique'].values()) for r in counts['rounds']) == len(terms)
    from jsonschema import Draft202012Validator
    schema = json.loads((ROOT / 'schemas/ruleset.schema.json').read_text())
    validator = Draft202012Validator(schema)
    for cat in (*CATEGORIES, 'all_categories'):
        rows = terms if cat == 'all_categories' else [t for t in terms if t['category'] == cat]
        actual = json.loads((root / 'rulesets' / (cat + '.ruleset.json')).read_text())
        assert actual == make_pack(rows)
        validator.validate(actual)
        if cat != 'all_categories':
            assert (root / 'words' / (cat + '.txt')).read_text().splitlines() == [t['word'] for t in rows]
    assert counts['default_bootstrap_changed'] is False and counts['active_rules_changed'] is False
    return {'status': 'PASS', 'rules': len(terms), 'categories': counts['counts'],
            'rounds_verified': 3, 'schema_packages': 5,
            'rust_and_network_validation': 'separate CI tests', 'business_accuracy': 'NOT_MEASURED'}


if __name__ == '__main__':
    try:
        print(json.dumps(validate(), ensure_ascii=False, indent=2))
    except (AssertionError, ValueError, OSError, ImportError) as exc:
        print('FAIL: ' + str(exc), file=sys.stderr)
        sys.exit(1)
