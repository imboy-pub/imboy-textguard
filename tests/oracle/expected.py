"""Oracle expectation scanner: computes expected hits for one corpus case.

Shared by the corpus generator (freezing expectations into fixtures) and by
the selftest comparator. Never touches production code.

Scene semantics come from the fixture ruleset: ``compat-v1`` scenes scan the
B view with canonical literals only (plan 4.2 - enhancement configuration
exists but never participates in the decision); ``enhanced-v1`` scenes add
t2s/gap/alias evidence.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gap_reference
import naive
import rulepool
import views

ACTION_RANK = {"review": 1, "block": 2}


def canonical_action(severity):
    return "block" if severity == "high" else "review"


def dedup_hits(hits):
    """Apply the plan 4.9 aggregation key: rule/variant/start/end keep the
    strongest action, then the fewest transforms, then fixed order."""
    best = {}
    for hit in hits:
        key = (hit["rule_id"], hit["variant_id"], hit["start"], hit["end"])
        rank = (ACTION_RANK[hit["action"]], -len(hit["methods"]))
        current = best.get(key)
        if current is None or rank > (ACTION_RANK[current["action"]],
                                      -len(current["methods"])):
            best[key] = hit
    return sorted(best.values(), key=lambda hit: (hit["rule_id"], hit["variant_id"],
                                                  hit["start"], hit["end"],
                                                  tuple(hit["methods"])))


def scan_case(case):
    """Return sorted, aggregated expected hits for a case per the contract."""
    text = case["text"]
    scene = case["scene"]
    semantics = rulepool.FIXTURE_RULESET["semantics_by_scene"][scene]
    enhanced = semantics == "enhanced-v1"
    b_units = views.base_units(text)
    t_units = (views.t2s_units(b_units, rulepool.TEST_T2S_TABLE)
               if case.get("t2s") and enhanced else None)
    hits = []
    for rule in rulepool.FIXTURE_RULESET["literals"]:
        if scene not in rule["scenes"]:
            continue
        word = rule["word"]
        action = canonical_action(rule["severity"])
        targets = [(b_units, ("literal",))]
        if t_units is not None:
            targets.append((t_units, ("literal", "t2s")))
        for target, methods in targets:
            pattern = views.base_units(word)
            if target is t_units:
                pattern = views.t2s_units(pattern, rulepool.TEST_T2S_TABLE)
            for hit in naive.scan_literal(rule["id"], pattern, target,
                                          methods=methods):
                hit["action"] = action
                hits.append(hit)
            if rule["gap"]["enabled"] and case.get("gap") and enhanced:
                gap_methods = (("literal", "gap") if target is b_units
                               else ("literal", "t2s", "gap"))
                for hit in gap_reference.scan_gap(rule["id"], pattern, target,
                                                  list(gap_methods)):
                    hit["action"] = rule["gap"]["action"]
                    hits.append(hit)
        if enhanced:
            for alias in rule["aliases"]:
                # aliases match the B view only, never t2s/gap (plan 4.5)
                pattern = views.base_units(alias["pattern"])
                for hit in naive.scan_alias(rule["id"], alias, pattern, b_units):
                    hit["action"] = alias["action"]
                    hits.append(hit)
    return dedup_hits(hits)


def compare_hits(expected, actual):
    """Comparator used by differential and mutation tests (AC-012)."""
    def key(hit):
        return (hit["rule_id"], hit["variant_id"], hit["start"], hit["end"],
                hit.get("action"), tuple(hit["methods"]))
    return [key(hit) for hit in expected] == [key(hit) for hit in actual]
