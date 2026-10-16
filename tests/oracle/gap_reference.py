"""Independent gap matcher reference (oracle lane, plan section 4.6).

Frozen defaults: ignorable characters are space, U+00B7, *, -, _; at most 2
ignored characters between adjacent effective characters; at most 4 ignored
characters inside one hit; hard boundaries are CR, LF, U+2028, U+2029,
., U+3002, !, U+FF01, ?, U+FF1F, ;, U+FF1B.

Only canonical literals may enable gap. The pattern itself (viewed) must not
contain ignorable or hard-boundary characters. The view is built by the B/T
normalization, so fullwidth boundary characters have already folded to their
ASCII forms before segmentation.
"""

GAP_IGNORABLE = frozenset(" \u00b7*-_")
HARD_BOUNDARIES = frozenset("\r\n\u2028\u2029.\u3002!\uff01?\uff1f;\uff1b")


def validate_gap_pattern(pattern_units):
    for unit in pattern_units:
        if unit.scalar in GAP_IGNORABLE or unit.scalar in HARD_BOUNDARIES:
            raise ValueError("gap pattern contains ignorable or boundary char")


def segments(units):
    """Split view units on hard boundaries; boundaries belong to no segment."""
    current = []
    for unit in units:
        if unit.scalar in HARD_BOUNDARIES:
            if current:
                yield current
            current = []
        else:
            current.append(unit)
    if current:
        yield current


def scan_gap(rule_id, pattern_units, text_units, methods):
    """All gap hits across every segment with adjacency/total budget checks.

    Gap counts are measured between adjacent effective characters inside the
    segment (their original positions), so deletions elsewhere never inflate
    a candidate's budget.
    """
    validate_gap_pattern(pattern_units)
    pattern = "".join(unit.scalar for unit in pattern_units)
    span = len(pattern)
    hits = []
    for segment in segments(text_units):
        effective = [(position, unit) for position, unit in enumerate(segment)
                     if unit.scalar not in GAP_IGNORABLE]
        body = "".join(unit.scalar for _, unit in effective)
        for index in range(0, len(body) - span + 1):
            if body[index:index + span] != pattern:
                continue
            window = effective[index:index + span]
            gaps = [window[next_index][0] - window[next_index - 1][0] - 1
                    for next_index in range(1, len(window))]
            if any(count > 2 for count in gaps):
                continue
            if sum(gaps) > 4:
                continue
            hits.append({"rule_id": rule_id, "variant_id": "canonical",
                         "start": window[0][1].start, "end": window[-1][1].end,
                         "methods": list(methods)})
    return hits
