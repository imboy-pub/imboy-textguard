# Round-8 review: code hygiene audit (2026-10-20, session4)

[中文](R8-code-hygiene-audit.md) | [English](R8-code-hygiene-audit.en.md)

## Rust candidate (49dfe3f, src/ production paths)
- dbg!: 0; println!: only main.rs:4 (--version output, a CLI feature); eprintln!: only the
  panic hook INTERNAL_ERROR and three startup-failure NOT_READY sites (deliberate failure observability)
- TODO/FIXME/HACK/XXX/DIAG: 0
- Historical item: the non-blocking T24 review note "residual DIAG eprintln" was cleared by
  commit 11dea3e within the candidate lineage (drop the leftover diagnostics block from ac088)

## Erlang candidate (a2fe60cd, src/moderation/)
- io:format / dbg( / TODO / FIXME / XXX: all 0

## Other spot checks
- Zero drift of candidate heads since round 7 (49dfe3f / a2fe60cd unchanged)
- CI workflow: fmt --check + clippy -D warnings + cargo test --locked + build --locked --release (pinned, real gates)
- Cross-check of deploy/README claims: ports 8081/8082/8083 with separated credentials ✓;
  check budget 80ms = runtime/mod.rs:15 CHECK_BUDGET constant ✓; /metrics on loopback only ✓

Zero defects, zero state changes.
