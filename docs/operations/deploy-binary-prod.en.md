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

## 6. Make the IMBoy backend use it (traffic switch, a separate operation)

The backend (Erlang, a bare process on the host) defaults to `mode=legacy` and does not pass
through TextGuard — after deployment the business is unaffected. Switching traffic requires
changing the backend's `moderation_detector` configuration (transport http, `127.0.0.1:8181`,
CHECK credential) and observing in shadow mode before switching; see
`priv/textguard/transport.example.config` in the repository.
**New block rules require business approval first** (currently a restricted-local posture).

## 7. Common failures

| Symptom | Cause and handling |
|---|---|
| Service restarts repeatedly, log shows `SECRET_FILE_UNREADABLE` | The secret files are not owned by `imboy-tg`; run `chown imboy-tg` on both |
| ready stays 503 | No ruleset published yet; follow §4 |
| Publish returns 400 INVALID_RULESET | Empty package missing `x-textguard-confirm-empty: true`, or a JSON field/value is invalid |
| Publish returns 503 UNSUPPORTED_RULESET | The ruleset's `t2s_table_id` / `normalization_id` differs from the service startup values |
| Publish returns 412 | If-Match is not the current etag; GET the current value again (idempotent 200 for identical content is normal) |
