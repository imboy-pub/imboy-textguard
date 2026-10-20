# TextGuard V2.1 delivery verification archive

[中文](README.md) | [English](README.en.md)

This directory is the **archived proof** of TextGuard MVP delivery verification (plan V2.1,
run-20261016T075111Z): that the artifacts are acceptable plus the basis for key engineering
decisions. The raw per-request test data (about 17G) was kept in an evidence area outside
the repository and was deleted on 2026-10-20 by user decision; this directory preserves all
adjudication files and the conclusion chain.

## Delivery status

`PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS` — TextGuard is green across its full scope; every
gap to a perfect score comes from pre-existing items outside the authorization or in the
user domain (see "Declared residuals").

| Gate | Result | Notes |
|---|---|---|
| G0–G4 baseline/engine/dual protocol/publication/business | PASS | Real Erlang↔Rust HTTP/gRPC loop |
| G5 resource capacity | PASS_WITH_DECLARED_SOAK_RESIDUAL | 500 QPS p99 3.9ms; 1500 QPS overload with zero unexpected; soak memory residual documented (allocator high-water mark, not a leak; A/B evidence 932→222MB) |
| G6 content | PASS_WITH_DECLARED_LIMITS | restricted_local: no business-authorized corpus, new block rules force review/shadow |
| G7 recovery | PASS | Clean install plus three rollback classes |
| G8 publication | PASS_WITH_DECLARED_RESIDUALS | Non-TextGuard pre-existing failures in full EUnit (22 native-gate items designed for GitHub runner only, plus 4 user-domain behavioural singletons) |
| G9 production | NOT_AUTHORIZED | Not authorized |
| Upstream U line | NO_UPSTREAM_CHANGE_NEEDED | No general aho-corasick defect; the product consumes the official release without patches |

## Key measured numbers (real Linux container, 2 vCPU / 2GiB)

- B1 enhanced baseline (50k canonical + 10k aliases + 5k gaps + 32 regex + 100 exceptions):
  500 completed requests/second, p99 3.9–4.7ms; overload ladder 750/1000/1500 QPS with
  unexpected=0 (the only 1500 failure is a bounded HTTP_429 rejection)
- One-hour soak: 1.8 million requests, p99 3.4ms, 100/100 ruleset publications
- Full EUnit (imboy f3ca4fbf): 24/24 chunks passed over two rounds, TextGuard 9/9 suites green
  (chunk-23 Passed:841)
- Runtime smoke (R4–R9): fail-closed startup, 401/400/503 semantics, 60 concurrent requests
  with zero errors, CAS three-way behaviour, gRPC wire 15/15, persistent recovery and
  disaster fail-closed, eight contract behaviours

## Candidates and lineage

- Service: `imboy-textguard` integration **49dfe3f** (aho-corasick =1.1.3 pinned from the
  registry; that branch has been merged into main=429293e, which additionally carries the
  user-requested upgrade to =1.1.5 — capacity evidence is not bound to 1.1.5)
- Consumer: `imboy` integration **a2fe60cd** (= main tip at merge time; migration
  00000197_textguard_review_metadata)
- Boundaries: not pushed, not submitted upstream (U05), production deployment not performed (T29)

## Declared residuals and unlock conditions

1. AC-095 soak memory residual: allocator high-water mark ~3.4MiB/pub (documented, not a leak);
2. G6: new block rules may only be enabled after the business-annotated corpus is authorized;
3. G8: run the native gate green on a GitHub runner, handle the 4 pre-existing user-domain
   singletons, then rerun full EUnit.

## File index

- `release-manifest.json` — candidate fingerprints / dependency pins / final gate states (authoritative)
- `runbook.md` — build/start/publish/detect/rollback/recover operations manual (every command actually run)
- `decisions-ledger.md` — execution decision log (contains historical references to evidence paths)
- `reviews/` — independent engineering reviews and nine rounds of review receipts (R4–R9 are
  runtime/wire/drift/hygiene audits)
- `evidence/` — key adjudication files: capacity observations, soak, A/B, corpus evaluation,
  EUnit summaries, upstream boundary, traceability snapshots
