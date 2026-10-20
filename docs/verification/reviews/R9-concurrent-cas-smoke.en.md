# Round-9 review: concurrent admission + CAS integrity smoke (2026-10-20, session4)

[中文](R9-concurrent-cas-smoke.md) | [English](R9-concurrent-cas-smoke.en.md)

Candidate 49dfe3f, real binary (macOS, fixture table, synthetic rules, secrets destroyed).

## Concurrent detection
60 concurrent requests (30 threads) with mixed text: 60/60 HTTP 200, each decision matching its
text one-to-one (allow 24 / block 12 / review 24), scan_complete true everywhere, zero 5xx,
service alive ✓

## CAS integrity (this round resolved an earlier misreading)
- Stale etag + identical content republished → 200 (idempotent for identical content, no state
  change, harmless)
- **Stale etag + different content → 412 CAS_CONFLICT rejection** ✓ (real conflict protection works)
- Stale etag + semantically invalid package (exception referencing a deleted rule) → 400 strict
  validation rejection ✓ (dangling-reference protection)
- Conclusion: the earlier round's "stale CAS 200" was the identical-content idempotent path, not
  a CAS defect; all three behaviours conform to the §9.1 contract

## Deliverable
- OPERATIONS-RUNBOOK.md: the executable command set required by §11.3 (build/start/publish/detect/
  rollback/recover/consumer), all R4–R9 measured commands, no secrets ✓

Resource cleanup complete, no state changes.
