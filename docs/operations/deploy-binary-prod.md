# TextGuard 二进制模式生产部署（新手向）

[中文](deploy-binary-prod.md) | [English](deploy-binary-prod.en.md)

> 目标：在一台 Linux（Debian 12，x86_64）服务器上以**裸二进制 + systemd** 方式部署
> TextGuard，约 30 分钟完成。全程不需要在服务器上装 Docker。

## 0. 前提

- 一台 Debian 12 x86_64 服务器，能 `ssh root@<服务器>` 登录
- 本机（你的 Mac/Linux 工作机）装有 Docker（只用于**构建**，服务器上不装）
- 已知示例值（本文以实际生产为准）：服务器 `106.53.76.53` SSH 端口 `32`

## 1. 构建 Linux 二进制（在工作机上执行，约 5-15 分钟）

```bash
cd /path/to/imboy-textguard
docker run --rm --platform linux/amd64 -v "$PWD":/src -w /src \
  -e TEXTGUARD_BUILD_ID=0.1.0-prod_49dfe3f \
  -e CARGO_TARGET_DIR=/src/target-x86_64 \
  rust:1.90-bookworm bash -c \
  "apt-get update -qq && apt-get install -y -qq protobuf-compiler && cargo build --locked --release && sha256sum target-x86_64/release/imboy-textguard"
# 产物：target-x86_64/release/imboy-textguard（记下 sha256）
```

> 原理：用 Debian 12 的 rust 容器编译，glibc 与服务器一致；`TEXTGUARD_BUILD_ID`
> 建议写成 `<版本>-prod_<git短sha>`，会出现在 `--version` 与 `/v1/check` 响应里。

## 2. 服务器准备（一次性，在服务器上执行）

```bash
ssh -p 32 root@106.53.76.53
useradd -r -s /usr/sbin/nologin -d /var/lib/imboy-textguard imboy-tg
mkdir -p /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin /var/lib/imboy-textguard/data /etc/imboy-textguard
# 两个访问凭证在服务器本地生成，不要通过任何聊天/工单传输
openssl rand -base64 48 | tr -d "\n" > /etc/imboy-textguard/check
openssl rand -base64 48 | tr -d "\n" > /etc/imboy-textguard/publish
chmod 600 /etc/imboy-textguard/check /etc/imboy-textguard/publish
chown imboy-tg:imboy-tg /etc/imboy-textguard/check /etc/imboy-textguard/publish
```

写环境文件 `/etc/imboy-textguard/env`（0600，root 属主即可）：

```ini
TEXTGUARD_DATA_DIR=/var/lib/imboy-textguard/data
TEXTGUARD_CHECK_SECRET_FILE=/etc/imboy-textguard/check
TEXTGUARD_PUBLISH_SECRET_FILE=/etc/imboy-textguard/publish
TEXTGUARD_HTTP_ADDR=127.0.0.1:8181
TEXTGUARD_GRPC_ADDR=127.0.0.1:8182
TEXTGUARD_MANAGE_ADDR=127.0.0.1:8183
TEXTGUARD_T2S_TABLE_ID=opencc-1.1.9-single-v1
```

写 systemd 单元 `/etc/systemd/system/imboy-textguard.service`：

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

## 3. 传输并启动

```bash
# 工作机上：
scp -P 32 target-x86_64/release/imboy-textguard root@106.53.76.53:/tmp/
# 服务器上：
echo "<你的sha256>  /tmp/imboy-textguard" | sha256sum -c -   # 校验
install -m 755 /tmp/imboy-textguard /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin/
rm /tmp/imboy-textguard
chown imboy-tg:imboy-tg /etc/imboy-textguard/check /etc/imboy-textguard/publish   # 关键！
systemctl enable --now imboy-textguard
systemctl is-active imboy-textguard        # 期望 active
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8181/health/live   # 期望 200
```

> ⚠️ 新手最常踩的坑：secret 文件必须 `chown imboy-tg`。忘了的话服务会一直重启，
> `journalctl -u imboy-textguard -n 3` 会显示 `NOT_READY: SECRET_FILE_UNREADABLE`。

## 4. 发布首个规则包（空包 = 只就绪、不拦截，业务零影响）

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
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready   # 期望 200
```

> ⚠️ 空规则包必须带 `x-textguard-confirm-empty: true` 头（防误发布全放行词库的保险），
> 否则返回 400 INVALID_RULESET。
> 验证检测：带 CHECK 凭证 POST /v1/check，应返回 `"decision":"allow"`。

## 5. 回滚 / 升级

- **回滚到旧二进制**：保留上一版目录（如 `/usr/local/imboy-textguard-0.1.0-prod_旧sha/`），
  改 service 的 ExecStart 指向后 `systemctl daemon-reload && systemctl restart imboy-textguard`。
  词库数据在 `/var/lib/imboy-textguard/data`，回滚二进制不影响已发布词库。
- **回滚词库**：用上一版规则包 JSON 重走 §4 的 PUT（If-Match 用当前 etag）。
- **升级**：重复 §1-§3，新版本放新目录，切换 ExecStart。

## 6. 让 IMBoy 后端使用它（业务切流，单独操作）

后端（Erlang，宿主机裸进程）默认 `mode=legacy`，不经过 TextGuard——部署完成后业务
零影响。切流需改后端配置 `moderation_detector`（transport http，`127.0.0.1:8181`，
CHECK 凭证）并在 shadow 模式观察后再切，参考仓库 `priv/textguard/transport.example.config`。
**新增 block 规则必须先取得业务批准**（当前为受限本地姿态）。

## 7. 常见故障

| 现象 | 原因与处理 |
|---|---|
| 服务反复重启，日志 `SECRET_FILE_UNREADABLE` | secret 文件属主不是 `imboy-tg`，`chown imboy-tg` 两个 secret 文件 |
| ready 一直是 503 | 尚未发布规则包，走 §4 |
| 发布 400 INVALID_RULESET | 空包缺 `x-textguard-confirm-empty: true`，或 JSON 字段/取值非法 |
| 发布 503 UNSUPPORTED_RULESET | 规则包 `t2s_table_id`/`normalization_id` 与服务启动值不一致 |
| 发布 412 | If-Match 不是当前 etag，重新 GET 取值（同内容幂等 200 属正常） |
