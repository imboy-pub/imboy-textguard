# T05 independent oracle

Run `./scripts/verify_textguard.sh --task T05 --evidence <new-directory>`.
This lane validates independent references and fixtures. Production candidate
comparison remains T08/T11; this is not full AC-013 product acceptance.

`unicode16.py` reads only the checked-in SHA-verified table under
`tests/fixtures/oracle/unicode16/`. It uses UnicodeData simple lowercase,
unconditional SpecialCasing full mappings, and default Final_Sigma with the
complete Cased/Case_Ignorable ranges. It never calls host Unicode casing or
properties, Rust, or another production implementation. Each original scalar
owns the full byte span of every output scalar it produces.

Every body, canonical rule, alias and exception explicitly supplies semantics.
`compat-v1` keeps the frozen OTP context-free mapping (ΟΣ → οσ);
`enhanced-v1` applies default context (ΟΣ → ος). Width folding and the specified
zero-width deletion happen after casing. Local exceptions are an independent
naive containment reference, scoped by scene/rule/variant/view, enabled only
for enhanced literal/alias candidates, excluding gap. Regex is outside T05.

Official input URLs, SHA-256 values and the derived table hash are pinned in
`unicode16/manifest.json`; the Unicode license is distributed beside the table.
The license is the official current text fetched on 2026-10-16, not a claimed
historical 2024 snapshot. Re-extract offline with official files:

```sh
python3 -B tests/oracle/extract_unicode16.py --source-dir <ucd-inputs> --check
```

Alternatively use `--download --check` to fetch the exact official inputs and
verify all hashes. These explicit source-audit commands access the network;
ordinary oracle tests are offline and do not need the original 3 MB UCD files.
The extractor fails on a hash change or an unsupported default casing context.

The 16 normalization anchors are hand-specified, including U+1C89, new-letter
Final_Sigma context, both semantics, expansions and byte provenance. Mutations
force the wrong body/rule/alias/exception semantics or remove the new Cased
property and must be detected. Original deleted/shifted/forged-hit mutations
remain. The original seed-20261016 10k corpus and manifest remain byte-identical;
regeneration is compared in memory with the frozen SHA, never written by tests.
