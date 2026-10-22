# TextGuard production deployment in binary mode (beginner-friendly)

[中文](deploy-binary-prod.md) | [English](deploy-binary-prod.en.md)

> Goal: deploy TextGuard as a **bare binary + systemd** on one Linux server (Debian 12,
> x86_64), roughly 30 minutes end to end. Docker is never required on the server.

## 0. Prerequisites

- One Debian 12 x86_64 server reachable via `ssh root@<server>`
- Docker on your own workstation (Mac/Linux), used for **building only** — not installed on the server
- Known example values (this document follows the actual production setup): server `106.53.76.53`, SSH port `32`

## 1. Build the Linux binary (on the workstation, about 5–15 minutes)

```bash
cd /path/to/imboy-textguard
docker run --rm --platform linux/amd64 -v "$PWD":/src -w /src \
  -e TEXTGUARD_BUILD_ID=0.1.0-prod_49dfe3f \
  -e CARGO_TARGET_DIR=/src/target-x86_64 \
  rust:1.90-bookworm bash -c \
  "apt-get update -qq && apt-get install -y -qq protobuf-compiler && cargo build --locked --release && sha256sum target-x86_64/release/imboy-textguard"
# Artifact: target-x86_64/release/imboy-textguard (record the sha256)
```

> Why: compiling inside a Debian 12 rust container keeps glibc aligned with the server.
> Set `TEXTGUARD_BUILD_ID` to `<version>-prod_<short-git-sha>`; it appears in `--version`
> and in `/v1/check` responses.

## 2. Server preparation (one-time, on the server)

```bash
ssh -p 32 root@106.53.76.53
useradd -r -s /usr/sbin/nologin -d /var/lib/imboy-textguard imboy-tg
mkdir -p /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin /var/lib/imboy-textguard/data /etc/imboy-textguard
# Both credentials are generated locally on the server; never send them through chat or tickets
openssl rand -base64 48 | tr -d "\n" > /etc/imboy-textguard/check
openssl rand -base64 48 | tr -d "\n" > /etc/imboy-textguard/publish
chmod 600 /etc/imboy-textguard/check /etc/imboy-textguard/publish
chown imboy-tg:imboy-tg /etc/imboy-textguard/check /etc/imboy-textguard/publish
```

Write the environment file `/etc/imboy-textguard/env` (0600, root-owned is fine):

```ini
TEXTGUARD_DATA_DIR=/var/lib/imboy-textguard/data
TEXTGUARD_CHECK_SECRET_FILE=/etc/imboy-textguard/check
TEXTGUARD_PUBLISH_SECRET_FILE=/etc/imboy-textguard/publish
TEXTGUARD_HTTP_ADDR=127.0.0.1:8181
TEXTGUARD_GRPC_ADDR=127.0.0.1:8182
TEXTGUARD_MANAGE_ADDR=127.0.0.1:8183
TEXTGUARD_T2S_TABLE_ID=opencc-1.1.9-single-v1
```

Write the systemd unit `/etc/systemd/system/imboy-textguard.service`:

```ini
[Unit]
Description=IMBoy TextGuard content-check service
After=network.target

[Service]
User=imboy-tg
Group=imboy-tg
EnvironmentFile=/etc/imboy-textguard/env
ExecStart=/usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin/imboy-textguard
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/imboy-textguard
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
```

## 3. Transfer and start

```bash
# On the workstation:
scp -P 32 target-x86_64/release/imboy-textguard root@106.53.76.53:/tmp/
# On the server:
echo "<your sha256>  /tmp/imboy-textguard" | sha256sum -c -   # verify
install -m 755 /tmp/imboy-textguard /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin/
rm /tmp/imboy-textguard
chown imboy-tg:imboy-tg /etc/imboy-textguard/check /etc/imboy-textguard/publish   # critical!
systemctl enable --now imboy-textguard
systemctl is-active imboy-textguard        # expect active
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8181/health/live   # expect 200
```

> Warning: the most common beginner trap is forgetting `chown imboy-tg` on the secret files.
> The service then restarts forever, and `journalctl -u imboy-textguard -n 3` shows
> `NOT_READY: SECRET_FILE_UNREADABLE`.

## 4. Publish the first ruleset (empty package = ready but blocking nothing, zero business impact)

```bash
cat > /etc/imboy-textguard/starter-ruleset.json <<'JSON'
{"schema_version":1,"normalization_id":"imboy-base-v1","t2s_table_id":"opencc-1.1.9-single-v1","limit_profile_id":"mvp-default-v1","semantics_by_scene":{"channel_message":"enhanced-v1","moment_post":"enhanced-v1","profile_field":"compat-v1"},"allow_empty":true,"literals":[],"regex_rules":[],"exceptions":[]}
JSON
PUB=$(cat /etc/imboy-textguard/publish)
ETAG=$(curl -s -H "authorization: Bearer $PUB" http://127.0.0.1:8183/internal/v1/rulesets/active | python3 -c "import sys,json;print(json.load(sys.stdin)['state_etag'])")
curl -s -X PUT http://127.0.0.1:8183/internal/v1/rulesets/active \
  -H "authorization: Bearer $PUB" -H "If-Match: $ETAG" \
  -H "x-textguard-confirm-empty: true" -H 'content-type: application/json' \
  --data-binary @/etc/imboy-textguard/starter-ruleset.json
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready   # expect 200
```

> Warning: an empty ruleset must carry the `x-textguard-confirm-empty: true` header (a guard
> against accidentally publishing an allow-everything ruleset); otherwise the server returns
> 400 INVALID_RULESET.
> Verify detection: POST /v1/check with the CHECK credential; it should return `"decision":"allow"`.

## 5. Rollback / upgrade

- **Roll back the binary**: keep the previous version directory (e.g. `/usr/local/imboy-textguard-0.1.0-prod_<old-sha>/`),
  point the unit's ExecStart at it, then `systemctl daemon-reload && systemctl restart imboy-textguard`.
  Rules data lives in `/var/lib/imboy-textguard/data`, so a binary rollback does not affect
  published rulesets.
- **Roll back the ruleset**: republish the previous ruleset JSON through the §4 PUT (with the current etag in If-Match).
- **Upgrade**: repeat §1–§3, place the new version in a new directory, switch ExecStart.

## 6. Make the IMBoy backend use it (traffic switch)

> The full manual-verification command set (curl/Python, copy-paste ready) lives in
> [verify-production.en.md](verify-production.en.md). This section covers how to switch
> and how to roll back.

### 6.0 What switching means, and who is affected

The backend (Erlang, bare process on the host) decides content moderation via the
`moderation_detector` config, with three modes:

| Mode | Behavior | When |
|---|---|---|
| `legacy` (default) | TextGuard fully bypassed; backend built-in word list | Not switched / rollback |
| `shadow` | Business behavior identical to legacy; TextGuard double-runs in the background recording diffs | Pre-switch observation |
| `rust` | Enforced | Target state |

**Scope**: profile fields only — `nickname` / `sign` / `profession` / `school` / `interests`
(code fact: `?PROFILE_REVIEW_FIELDS` in `imboy/src/logic/user_logic.erl`).

**Verdicts**: `high` hit → save rejected; `medium`/`low` hit → **save succeeds + entry in the
admin review queue**; TextGuard failure/timeout → fail-open (`failure_policy=availability_compat`).
The 2026-10 lexicon is all `medium`, so after switching, nickname saves always succeed —
hits simply land in the review queue.

### 6.1 Pre-flight (30 seconds)

```bash
curl -s -o /dev/null -w "live:%{http_code} "  http://127.0.0.1:8181/health/live
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready   # both must be 200
```

### 6.2 The config block (what each key means)

```erlang
{moderation_detector, [
    {mode, rust},                          %% legacy | shadow | rust
    {transport, http},                     %% HTTP adapter
    {failure_policy, availability_compat}, %% fail-open on detector failure
    {http_base_url, <<"http://127.0.0.1:8181">>},  %% TextGuard check port
    {timeout_ms, 150},                     %% per-check deadline (TextGuard internal budget 80ms)
    {max_response_bytes, 262144},          %% response cap (default 65536; raise with a large ruleset)
    {token, <<"<CHECK credential>">>}      %% contents of /etc/imboy-textguard/check from §2
]}
```

A missing/invalid `token` makes the adapter return `not_ready` (fail-closed config load);
combined with `availability_compat` this means "allow but log" — never silently blocks business.
Publish-side keys (`publish_http_base_url`/`publish_token`) are not needed by the backend.

### 6.3 Hot switch (no downtime, shadow → rust)

On the server, locate the running node and cookie first:

```bash
ss -tlnp | grep ':9800 '                         # note the pid
tr '\0' ' ' < /proc/<pid>/cmdline | cut -c1-200  # note /usr/local/imboy-<ver>-prod_<sha>
REL=/usr/local/imboy-1.0.0-alpha.82-prod_70a839f6        # ← replace with the actual dir
NODE=$(grep '^-name' $REL/releases/*/vm.args | awk '{print $2}')
COOKIE=$(grep '^-setcookie' $REL/releases/*/vm.args | awk '{print $2}')
ERL=/usr/local/lib/erlang/bin/erl_call
```

Step 1 — write the config and enter shadow (observation; zero business change).
The credential is read server-side only:

```bash
TOKEN=$(cat /etc/imboy-textguard/check)
printf 'ok = application:set_env(imboy, moderation_detector, [{mode,shadow},{transport,http},{failure_policy,availability_compat},{http_base_url,<<"http://127.0.0.1:8181">>},{timeout_ms,150},{max_response_bytes,262144},{token,<<"%s">>}]), moderation_shadow:set_mode(shadow).\n' "$TOKEN" \
  | $ERL -s -name $NODE -c "$COOKIE" -e
$ERL -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"   # expect mode => shadow
```

After observing without anomalies, step 2 — flip to rust:

```bash
printf 'moderation_shadow:set_mode(rust).\n' | $ERL -s -name $NODE -c "$COOKIE" -e
$ERL -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"   # expect mode => rust
```

`set_mode/1` updates both the running node's application env and ETS — immediate, no restart.
Verify the verdict with a business-entry probe (expect `{queued, [...]}` for a lexicon word):

```bash
printf 'W = <<"0", 16#E5,16#94,16#AE, "k", 16#E7,16#B2,16#89, "qq">>, V = moderation_business:inspect_profile(<<"nickname">>, W), io:format("verdict:~p~n", [V]).\n' \
  | $ERL -s -name $NODE -c "$COOKIE" -fetch_stdout -e
%% expect verdict:{queued,[...hits with category => <<"drugs">> ...]}
```

### 6.4 Persist (survive node restarts)

The hot switch lives in memory only; **after a restart the release dir's sys.config wins**.
Insert the §6.2 block into `$REL/releases/<ver>/sys.config` right after `{imboy, [`,
then validate the syntax (a broken sys.config breaks the next boot):

```bash
CFG=$REL/releases/1.0.0-alpha.82/sys.config      # adjust the version
cp "$CFG" "$CFG.bak.$(date +%Y%m%dT%H%M%S)"      # back up first
# ...insert the block (read token from /etc/imboy-textguard/check; never hand-copy)...
erl -noshell -eval "case file:consult(\"$CFG\") of {ok,_} -> io:format(\"PARSE_OK~n\"); E -> io:format(\"FAIL:~p~n\",[E]) end" -s init stop
```

Blue-green releases do **not** inherit this block automatically — either carry it in the
deployment config source, or redo §6.3 + §6.4 after each release.

### 6.5 Rollback (three levels, fastest first)

1. **Instant hot rollback**: `printf 'moderation_shadow:set_mode(legacy).\n' | $ERL -s -name $NODE -c "$COOKIE" -e`
2. **Persistent rollback**: set `mode` to `legacy` in sys.config (or delete the block),
   re-run the `file:consult` validation; the next restart boots legacy.
3. **Full decommission**: `systemctl stop imboy-textguard && systemctl disable imboy-textguard`.
   With backend fail-open the business is unaffected; only detection is lost.

### 6.6 Known pitfalls (hit in practice)

- `erl_call` takes the cookie via **`-c`**, not `-cookie`.
- `erl_call -a` argument parsing **does not accept binary literals** — calls containing
  `<<"...">>` must go through `-e` (stdin), **with a trailing newline**.
- Writing `<<"中文"/utf8>>` in an erl_call probe gets **double-encoded** by the node's latin1
  parser, producing a false "no hit"; the real App path (Cowboy JSON parsing) is unaffected.
  Probe with the byte-exact form in §6.3 or the curl in [verify-production.en.md](verify-production.en.md).
- A `version_incomparable` counter during shadow observation is **expected** (backend built-in
  word list vs TextGuard lexicon are naturally different versions), not a fault.

## 7. Common failures

| Symptom | Cause and handling |
|---|---|
| Service restarts repeatedly, log shows `SECRET_FILE_UNREADABLE` | The secret files are not owned by `imboy-tg`; run `chown imboy-tg` on both |
| ready stays 503 | No ruleset published yet; follow §4 |
| Publish returns 400 INVALID_RULESET | Empty package missing `x-textguard-confirm-empty: true`, or a JSON field/value is invalid |
| Publish returns 503 UNSUPPORTED_RULESET | The ruleset's `t2s_table_id` / `normalization_id` differs from the service startup values |
| Publish returns 412 | If-Match is not the current etag; GET the current value again (idempotent 200 for identical content is normal) |
| After switching, every check allows | TextGuard stopped, or `token`/`http_base_url` wrong → fail-open; check `systemctl is-active imboy-textguard` and the §6.2 config |
| `status` shows legacy although the hot switch was done | The node restarted without the block in sys.config (hot switch is memory-only); redo §6.3 + §6.4 |
