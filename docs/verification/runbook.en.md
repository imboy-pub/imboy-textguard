# TextGuard delivery operations manual (consolidated, 2026-10-20)

[中文](runbook.md) | [English](runbook.en.md)

Source: consolidated from the R4–R9 smoke-test commands actually executed. Every command below
has been run and verified. No secrets are included.

## 0. Candidates
- Service: `imboy-textguard` repo, `textguard/run-20261016T075111Z/integration` (49dfe3f)
- Consumer: `imboy` repo, `textguard/run-20261016T075111Z/imboy-integration` (a2fe60cd)

## 1. Build (repository root)
```bash
cd /Users/leeyi/project/imboy.pub/imboy-textguard
git checkout textguard/run-20261016T075111Z/integration
cargo build --locked --release            # about 21s
cargo test --locked --test contract       # 7/7
cargo test --locked --test grpc_contract  # 15/15 (real TCP)
cargo fmt --all -- --check && cargo clippy --locked --all-targets -- -D warnings
```

## 2. Start (prepare 0600 secrets and an empty rules directory first)
```bash
SEC=/Users/leeyi/tg-secrets; mkdir -p $SEC/rules
python3 -c "import secrets,string;a=string.ascii_letters+string.digits;print(''.join(secrets.choice(a) for _ in range(48)))" > $SEC/check
python3 -c "import secrets,string;a=string.ascii_letters+string.digits;print(''.join(secrets.choice(a) for _ in range(48)))" > $SEC/publish
chmod 600 $SEC/check $SEC/publish
# Note: no ancestor of the data directory may be a symlink (symlink hardening; /tmp does not work on macOS)
TEXTGUARD_T2S_TABLE_ID=opencc-1.1.9-single-v1 \
TEXTGUARD_DATA_DIR=$SEC/rules TEXTGUARD_CHECK_SECRET_FILE=$SEC/check \
TEXTGUARD_PUBLISH_SECRET_FILE=$SEC/publish ./target/release/imboy-textguard
# Ports: HTTP 8181 / gRPC 8182 / management 8183 (all loopback)
```
Verify immediately: `curl -s -o /dev/null -w %{http_code} http://127.0.0.1:8181/health/live` → 200; `/health/ready` is 503 before publication.

## 3. Publish a ruleset (CAS)
```bash
PUB=$(cat $SEC/publish)
ETAG=$(curl -s -H "authorization: Bearer $PUB" http://127.0.0.1:8183/internal/v1/rulesets/active | python3 -c "import sys,json;print(json.load(sys.stdin)['state_etag'])")
curl -s -X PUT http://127.0.0.1:8183/internal/v1/rulesets/active \
  -H "authorization: Bearer $PUB" -H "If-Match: $ETAG" -H 'content-type: application/json' \
  --data-binary @<ruleset>.json     # t2s_table_id must match the startup table, otherwise UNSUPPORTED_RULESET
```
Behaviour: stale etag + identical content → idempotent 200; stale etag + different content → 412 CAS_CONFLICT; invalid package (e.g. an exception referencing a missing rule) → 400.

## 4. Detection
```bash
CHK=$(cat $SEC/check)
curl -s -X POST http://127.0.0.1:8181/v1/check -H "authorization: Bearer $CHK" \
  -H 'content-type: application/json' \
  -d '{"request_id":"demo-1","scene":"channel_message","fields":[{"field_id":"f1","text":"…"}]}'
# unauthenticated → 401; bad body → 400; not published → 503 NOT_READY (never allow)
```

## 5. Rollback = republish the previous ruleset (same as §3, old package as body, current etag in If-Match)
Publication history lives in `publication-history/` under the data directory; if ACTIVE is
corrupted, restart → ready 503 fail-closed (measured).

## 6. Consumer side (imboy repo)
```bash
cd /Users/leeyi/project/imboy.pub/imboy
git checkout textguard/run-20261016T075111Z/imboy-integration
# TextGuard EUnit evidence: runs/run-20261016T075111Z/T27-eunit-attempt4 (TextGuard 9/9 green)
scripts/verify_textguard.sh   # needs the evidence environment configured per the frozen commands.json; BLOCKED/exit 2 without it (by design)
```

## 7. Boundaries
- Production T29 and public submission U05 are not authorized and must not be executed.
- Enablement limit: without a business-authorized corpus, new block rules are forced into
  review/shadow (G6 restricted_local).
