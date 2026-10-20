# Round-6 review: persistent recovery + gRPC wire smoke receipt (2026-10-20, session4)

[中文](R6-recovery-grpc-smoke.md) | [English](R6-recovery-grpc-smoke.en.md)

Candidate 49dfe3f, real binary (macOS, fixture table, synthetic rules, temporary secrets
destroyed).

## gRPC wire level
tests/grpc_contract.rs (real TcpListener + tonic Channel over the network): 15 passed / 0 failed ✓

## §9.4 persistent recovery
- Publish generation 1 (rev 5fea107f..., instance 782520a7...) → kill the process → restart on
  the same data directory: ready=200 (no republish needed), revision exactly identical,
  instance-id SAME, detection still works (test word → block/1001) ✓
## Disaster fail-closed
- Delete the ACTIVE manifest (manifest.json) → restart: ready=503, detection returns NOT_READY
  retryable — no crash, never returns allow ✓

Resource cleanup complete, no state changes.
