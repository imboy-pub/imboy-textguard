# Round-7 review: TextGuard drift audit on the imboy product side (2026-10-20, session4)

[中文](R7-imboy-drift-audit.md) | [English](R7-imboy-drift-audit.en.md)

Scope: f3ca4fbf (attempt4 EUnit evidence binding head) → a2fe60cd (imboy integration
candidate head).

## Facts
- 16 commits between the two; the only TextGuard-related path change is that `src/textguard_pb.erl`
  was newly tracked (+10872 lines, purely additive; `src/textguard.proto` unchanged).
- 35b41e78 (textguard migration renumbered to 00000197) is an ancestor of the candidate ✓
  (merged before f3ca4fbf; already covered by the evidence).

## Proof of functional equivalence for the generated file
- The committed pb (1b813e99) ≠ the Makefile canonical regeneration (397c20bc, byte-identical
  to the EUnit runtime regeneration local-regen): the difference is export grouping layout and
  version comments (different gpb invocation paths, both gpb 5.0.0).
- Full exported-function-set diff = empty; message representation is maps in both;
- **Decisive**: CheckTextRequest (with enum + repeated nesting + UTF-8 text) encodes to
  **byte-identical** output.
- Conclusion: zero functional drift; EUnit evidence substantively covers the candidate head.
  The manifest statement evidence_binding_head=f3ca4fbf stays honest, and this round adds the
  equivalence proof.

## Hygiene note (user domain, non-blocking)
- The committed pb comes from a non-Makefile invocation path (missing the standard
  use_packages artifact layout); the user is advised to regenerate via make so the bytes are
  canonically tracked. This is not a delivery defect.

## Frozen verification entrypoint
- verify_textguard.sh --list returns BLOCKED + exit code 2 without an evidence environment,
  exactly per §15.3; during delivery it is driven by the frozen commands.json ✓

## Self-correction record
- During the audit my probe used the wrong message name (CheckRequest; the real one is
  CheckTextRequest), causing four false case_clause reports. Corrected — probe error, not a
  product defect.

Resource cleanup complete, no state changes.
