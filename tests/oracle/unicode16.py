"""Independent default lowercase from official Unicode 16 mappings/properties.

No host Unicode casing, property predicates, normalization, or production code.
Locale-specific SpecialCasing entries do not apply to either semantic mode.
"""

from bisect import bisect_right
import hashlib
import json
from pathlib import Path

DATA = Path(__file__).resolve().parent.parent / "fixtures/oracle/unicode16"
MANIFEST = json.loads((DATA / "manifest.json").read_bytes())
PAYLOAD = (DATA / "lowercase.json").read_bytes()
if hashlib.sha256(PAYLOAD).hexdigest() != MANIFEST["table_sha256"]:
    raise ValueError("Unicode 16 oracle table checksum mismatch")
TABLE = json.loads(PAYLOAD)
if TABLE["unicode_version"] != "16.0.0":
    raise ValueError("Unicode 16 oracle version mismatch")
LOWER = {int(point): mapping for point, mapping in TABLE["lower"].items()}
CONTEXTS = {int(point): entry for point, entry in TABLE["contexts"].items()}
RANGES = TABLE["properties"]
STARTS = {name: [bounds[0] for bounds in ranges]
          for name, ranges in RANGES.items()}


def has_property(point, name):
    position = bisect_right(STARTS[name], point) - 1
    return position >= 0 and point <= RANGES[name][position][1]


def final_sigma(points, index):
    before = index - 1
    while before >= 0 and has_property(points[before], "Case_Ignorable"):
        before -= 1
    if before < 0 or not has_property(points[before], "Cased"):
        return False
    after = index + 1
    while after < len(points) and has_property(points[after], "Case_Ignorable"):
        after += 1
    return after == len(points) or not has_property(points[after], "Cased")


def lowercase_parts(text, semantics):
    """One independently calculated output string per original scalar."""
    if semantics not in {"compat-v1", "enhanced-v1"}:
        raise ValueError("unknown oracle semantics")
    text.encode("utf-8", errors="strict")
    points = [ord(scalar) for scalar in text]
    result = []
    for index, point in enumerate(points):
        mapping = LOWER.get(point, [point])
        context = CONTEXTS.get(point)
        if semantics == "enhanced-v1" and context:
            if context["condition"] != "Final_Sigma":
                raise ValueError("unsupported Unicode context")
            if final_sigma(points, index):
                mapping = context["lower"]
        result.append("".join(chr(value) for value in mapping))
    return result


def lowercase(text, semantics):
    return "".join(lowercase_parts(text, semantics))
