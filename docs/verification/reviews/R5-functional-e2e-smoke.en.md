# Round-5 review: functional end-to-end smoke receipt (2026-10-20, session4)

[中文](R5-functional-e2e-smoke.md) | [English](R5-functional-e2e-smoke.en.md)

Candidate 49dfe3f, real binary (macOS, fixture table test-t2s-v1, synthetic rules, temporary
secrets destroyed).

## Publication loop
- Fetch empty-store etag → PUT (If-Match CAS) → generation 1 / ready:true / scene_counts 3×3 ✓; ready flips 503→200 ✓
- Snapshot identity contract: ruleset `t2s_table_id` not matching the service startup table → rejected with UNSUPPORTED_RULESET ✓ (409 also rejects missing CAS/If-Match)
- Ruleset table mismatch resolved by restarting with the real table, after which publication succeeded — identity validation works as designed

## Detection probes (all contracts correct)
1. Alias `ceshici` (rule severity=high, alias action=review) → decision=review, effective_action=review — decision/action separated from severity ✓
2. Canonical test word with high → block, start_byte=6 original-text byte offset ✓
3. Canonical demo word with medium → review ✓
4. Exception view scope: phrase demo-word-report with views=["B"] → the hit survives through T-view witness (review, match_methods=[literal,t2s]); widening to views=["B","T"] → allow hits=0 — the contract semantics "an exception must not blanket-release a span, it exempts precisely per view" proven at runtime ✓
5. No hit → allow, scan_complete=true ✓
6. Dual semantics: profile_field(compat-v1) leaves the alias uncompiled (allow) while canonical still blocks; channel_message(enhanced-v1) activates both ✓

## Three-repository drift audit
- No moderation-contract path changes in imboyapp since 2215ef2d nor in imboyadmin since a2160f46 — UNCHANGED_VERIFIED holds at the current HEAD ✓

## Disposition
- Zero defects, zero state changes. Review worktree/secrets cleaned up.
