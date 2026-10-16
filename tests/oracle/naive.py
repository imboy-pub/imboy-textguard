"""Naive full-occurrence literal matcher (oracle lane).

Finds every occurrence of every pattern, including overlapping ones, by direct
slice comparison. This is intentionally O(n*m): the point is independence from
the Aho-Corasick production engine, not speed.
"""

from views import Unit, units_text


def find_all(pattern_units, text_units):
    """Yield (start_unit_index, end_unit_index) for every overlapping hit."""
    pattern = units_text(pattern_units)
    text = units_text(text_units)
    if not pattern:
        raise ValueError("empty pattern is not a legal rule")
    span = len(pattern)
    for index in range(0, len(text) - span + 1):
        if text[index:index + span] == pattern:
            yield index, index + span


def hit_range(text_units, start_index, end_index):
    """Original-text byte range [start, end) with conservative coverage."""
    first = text_units[start_index]
    last = text_units[end_index - 1]
    return first.start, last.end


def scan_literal(rule_id, pattern_units, text_units, methods=("literal",)):
    """All literal hits of one pattern with provenance-resolved byte ranges."""
    hits = []
    for start_index, end_index in find_all(pattern_units, text_units):
        start, end = hit_range(text_units, start_index, end_index)
        hits.append({"rule_id": rule_id, "variant_id": "canonical",
                     "start": start, "end": end, "methods": list(methods)})
    return hits


def ascii_word_boundary_ok(text_units, index, span):
    """ASCII word boundary required for pinyin-style aliases (plan 4.5).

    Neighbouring characters must not be [A-Za-z0-9_]; Chinese neighbours are
    allowed. Checked on the B view.
    """
    left_ok = index == 0 or not _is_word_byte(text_units[index - 1].scalar)
    right_end = index + span
    right_ok = (right_end >= len(text_units)
                or not _is_word_byte(text_units[right_end].scalar))
    return left_ok and right_ok


def _is_word_byte(scalar):
    return scalar.isascii() and (scalar.isalnum() or scalar == "_")


def scan_alias(rule_id, alias, pattern_units, text_units):
    """All alias hits; boundary=ascii_word applies the ASCII fence."""
    hits = []
    for start_index, end_index in find_all(pattern_units, text_units):
        if alias["boundary"] == "ascii_word" and not ascii_word_boundary_ok(
                text_units, start_index, end_index - start_index):
            continue
        start, end = hit_range(text_units, start_index, end_index)
        hits.append({"rule_id": rule_id, "variant_id": alias["variant_id"],
                     "start": start, "end": end, "methods": ["alias"]})
    return hits
