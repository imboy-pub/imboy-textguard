# Round-4 review: runtime smoke receipt (2026-10-20 ~06:30, session4)

[中文](R4-startability-smoke.md) | [English](R4-startability-smoke.en.md)

Candidate: imboy-textguard integration@49dfe3f (aho-corasick =1.1.3 from registry), macOS
development machine (startability only — not a capacity measurement).

- Build: `cargo build --locked --release` passed in 21.2s; `cargo test --locked --test contract` 7 passed / 0 failed.
- Startup (temporary 0600 secrets + empty rules directory, since destroyed):
  - unauthenticated POST /v1/check → 401 ✓
  - malformed body → 400 INVALID_ARGUMENT (`deny_unknown_fields` protocol validation) ✓
  - valid body + nothing published → **503 NOT_READY retryable:true (never returns allow)** ✓
  - unauthenticated management GET /internal/v1/rulesets/active → 401 ✓
  - /health/ready → 503; /health/live → 200 ✓
  - gRPC 8082 reachable over TCP ✓; store layout (instance-id 0600/packages/tmp) created correctly ✓
- Unplanned finding: startup is refused when the data directory sits under /tmp (a symlinked
  ancestor) with RULE_STORE_UNAVAILABLE — the publication-store symlink hardening was proven
  at real runtime ✓
- Cleanup: review worktree and temporary secrets deleted.
