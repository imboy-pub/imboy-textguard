"""Independent reference views for the oracle lane (T05).

This module is deliberately independent of the Rust production normalizer:
expectations frozen here are later compared against the engine in the T08/T11
differentials. Reference semantics per plan section 4.3:

    validate UTF-8 -> pinned Unicode 16 lowercase -> U+FF01..U+FF5E to
    halfwidth -> delete U+200B/U+200C/U+200D/U+FEFF

The OTP compatibility flavor (``otp_compat_lower``) reproduces Erlang
``string:lowercase/1`` on the frozen probe vectors: per-character full
lowercase WITHOUT the Final_Sigma context rule. Both flavors are exercised by
the frozen vectors so T06 can implement and verify each side. The enhanced
flavor applies the default Final_Sigma context using official UCD properties.
"""

import unicode16

ZERO_WIDTH = frozenset("\u200b\u200c\u200d\ufeff")


class Unit:
    """One view character carrying the conservative original byte range."""

    __slots__ = ("scalar", "start", "end")

    def __init__(self, scalar, start, end):
        self.scalar = scalar
        self.start = start
        self.end = end

    def __repr__(self):  # pragma: no cover - debugging aid
        return f"Unit({self.scalar!r},{self.start},{self.end})"


def _fold_fullwidth(scalar):
    point = ord(scalar)
    if 0xFF01 <= point <= 0xFF5E:
        return chr(point - 0xFEE0)
    return scalar


class NormalizerIncompatible(Exception):
    """Raised when source provenance cannot be proven for an input."""


def otp_compat_lower(text):
    """Erlang string:lowercase flavor: per-char full mapping, no context rules."""
    return unicode16.lowercase(text, "compat-v1")


def base_units(text, semantics):
    """Build the B view of ``text`` with per-character provenance.

    Each source scalar owns all of its mapped scalars, including expansions.
    Casing is computed directly from UCD, not aligned to a host lower result.
    """
    lowered_units = []
    byte_cursor = 0
    for character, mapped in zip(text, unicode16.lowercase_parts(text, semantics)):
        byte_end = byte_cursor + len(character.encode("utf-8"))
        for out in mapped:
            lowered_units.append(Unit(out, byte_cursor, byte_end))
        byte_cursor = byte_end
    units = []
    for unit in lowered_units:
        folded = _fold_fullwidth(unit.scalar)
        if folded in ZERO_WIDTH:
            continue
        units.append(Unit(folded, unit.start, unit.end))
    return units


def units_text(units):
    return "".join(unit.scalar for unit in units)


def t2s_units(units, table):
    """Apply the restricted single-scalar t2s table; unknown chars pass through."""
    return [Unit(table.get(unit.scalar, unit.scalar), unit.start, unit.end)
            for unit in units]
