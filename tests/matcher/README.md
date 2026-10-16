# T08 literal/alias component

Run `cargo test --locked --offline --test matcher -- --nocapture` with an isolated
`CARGO_TARGET_DIR`. Discovery uses the same command with `-- --list`. The driver
uses the exported library matcher (25 tests). T08 also registers the library
aggregation and two expansion-budget checks (3 tests), selected with
`cargo test --locked --lib matcher:: -- --skip matcher::regex::`.

The differential test reads the frozen T05 corpus, verifies its manifest SHA256
and seed 20261016, and compares all 10,000 cases against the actual matcher. It
projects only literal/alias witnesses out of the frozen expectations. Gap
witnesses are counted as deferred to T09/T11; they are neither fabricated nor
marked passed. Regex and final request aggregation belong to T10/T11. The
`oracle-ruleset.json` file is an exact JSON snapshot of the synthetic data in
`tests/oracle/rulepool.py`; expected hits never come from production code.

`CompiledLiterals::compile` returns a static, non-content `CompileError::issue`;
`ErrorCode::from(error)` is the publication adapter. Compat B, enhanced B and
enhanced T canonical patterns use the corresponding real normalizer. Aliases
are enhanced B only. A logical canonical contributes three compiled entries,
an alias one; every metadata entry and its normalized bytes count before
normalized-text deduplication. Fanout is checked separately per automaton.

`scan_text(text, scene, enable_t2s, budget, cancel)` is a component convenience
API. `scan_base` and `scan_t2s` accept provenance units for the T11 pipeline.
One `CandidateBudget` must span the entire request: every field, B/T/G, metadata
expansion and exception occurrence. A matched pattern consumes one candidate
per metadata entry, before scene or boundary filtering; raw and expanded work
are not charged twice. G/exception consumers must use the same `consume` API.
At most 20,000 candidates succeed; consuming 20,001 returns an error. Candidate
budget failures and cancellation observed through the budget remain terminal.
There is no partial-hit success on an interrupted scan. Request-level response
sorting, field IDs, metadata enrichment, exception suppression and the final
100-hit cap remain T11 obligations.

The component tests include the OTP-compatible `ΟΣ -> οσ` behavior and the
enhanced full-string `ΟΣ -> ος` behavior. They do not alter T06's old vectors.
