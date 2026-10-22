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

## 6. 让 IMBoy 后端使用它（业务切流）

> 完整的人工验证命令集（curl/Python，复制即用）见
> [verify-production.md](verify-production.md)。本节讲怎么切、怎么回退。

### 6.0 切流是什么、影响谁

后端（Erlang，宿主机裸进程）通过 `moderation_detector` 配置决定内容审核走向，共三档模式：

| 模式 | 行为 | 何时用 |
|---|---|---|
| `legacy`（默认） | 完全旁路 TextGuard，走后端内置词表 | 未切流 / 回退 |
| `shadow` | 业务行为与 legacy 完全相同，后台双跑 TextGuard 记差异 | 切流前观察 |
| `rust` | 真实生效 | 目标状态 |

**生效范围**：仅资料字段 `nickname` / `sign` / `profession` / `school` / `interests`
（代码事实：`imboy/src/logic/user_logic.erl` 的 `?PROFILE_REVIEW_FIELDS`）。

**判定行为**：命中 `high` → 拒绝保存；命中 `medium`/`low` → **保存成功 + 进管理后台复核队列**；
TextGuard 故障/超时 → fail-open 放行（`failure_policy=availability_compat`，检测挂了不挡业务）。
2026-10 词库 8851 条全部为 `medium`，所以切流后用户改昵称等**保存都会成功**，只是命中词进复核队列。

### 6.1 前置确认（切流前 30 秒自查）

```bash
curl -s -o /dev/null -w "live:%{http_code} "  http://127.0.0.1:8181/health/live
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready   # 都必须是 200
```

### 6.2 配置段内容（每个键的含义）

后端通过 `application env` 的 `imboy → moderation_detector` 读取配置，完整清单：

```erlang
{moderation_detector, [
    {mode, rust},                          %% legacy | shadow | rust
    {transport, http},                     %% 检测走 HTTP 适配层
    {failure_policy, availability_compat}, %% 检测故障时 fail-open 放行
    {http_base_url, <<"http://127.0.0.1:8181">>},  %% TextGuard 检测口
    {timeout_ms, 150},                     %% 单次检测总 deadline（TextGuard 内部预算 80ms）
    {max_response_bytes, 262144},          %% 响应体上限（默认 65536，规则包大后建议放宽）
    {token, <<"<CHECK 凭证>">>}            %% §2 生成的 /etc/imboy-textguard/check 的内容
]}
```

注意：`token` 缺失或非法时适配层直接返回 `not_ready`（fail-closed 配置装载），
配合 `availability_compat` 表现为"放行但记错误"——**不会静默硬拦，也不会挡业务**。
发布面（`publish_http_base_url`/`publish_token`）后端不需要，不配。

### 6.3 热切换（不停服，shadow → rust 两步走）

在服务器上执行。先定位运行节点与 cookie：

```bash
# 找到占用 9800 的进程，从 cmdline 里读出 release 目录
ss -tlnp | grep ':9800 '                       # 记下 pid
tr '\0' ' ' < /proc/<pid>/cmdline | cut -c1-200  # 记下 /usr/local/imboy-<版本>-prod_<sha>
REL=/usr/local/imboy-1.0.0-alpha.82-prod_70a839f6        # ← 换成上面读到的
NODE=$(grep '^-name' $REL/releases/*/vm.args | awk '{print $2}')
COOKIE=$(grep '^-setcookie' $REL/releases/*/vm.args | awk '{print $2}')
ERL=/usr/local/lib/erlang/bin/erl_call
```

第一步：写入配置并进入 shadow（观察期，业务零变化）。凭证只在服务器侧读取：

```bash
TOKEN=$(cat /etc/imboy-textguard/check)
printf 'ok = application:set_env(imboy, moderation_detector, [{mode,shadow},{transport,http},{failure_policy,availability_compat},{http_base_url,<<"http://127.0.0.1:8181">>},{timeout_ms,150},{max_response_bytes,262144},{token,<<"%s">>}]), moderation_shadow:set_mode(shadow).\n' "$TOKEN" \
  | $ERL -s -name $NODE -c "$COOKIE" -e
$ERL -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"   # 期望 mode => shadow
```

观察无异常后，第二步翻到 rust（生效）：

```bash
printf 'moderation_shadow:set_mode(rust).\n' | $ERL -s -name $NODE -c "$COOKIE" -e
$ERL -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"   # 期望 mode => rust
```

`set_mode/1` 会同时更新运行节点的 application env 和 ETS，立即生效，无需重启。
验证判定（在 rust 模式下用业务入口探一次，期望命中词返回 `{queued, [...]}`）：

```bash
printf 'W = <<"0", 16#E5,16#94,16#AE, "k", 16#E7,16#B2,16#89, "qq">>, V = moderation_business:inspect_profile(<<"nickname">>, W), io:format("verdict:~p~n", [V]).\n' \
  | $ERL -s -name $NODE -c "$COOKIE" -fetch_stdout -e
%% 期望 verdict:{queued,[...hits 里 category => <<"drugs">> ...]}
```

### 6.4 持久化（防节点重启后回退）

§6.3 的热切换只活在内存里；**节点重启后以 release 目录的 sys.config 为准**。
把 §6.2 的配置段插入 `$REL/releases/<版本>/sys.config` 的 `{imboy, [` 之后（即 imboy
段落列表的开头），然后必须校验语法（sys.config 损坏会导致下次启动失败）：

```bash
CFG=$REL/releases/1.0.0-alpha.82/sys.config      # 版本号按实际
cp "$CFG" "$CFG.bak.$(date +%Y%m%dT%H%M%S)"      # 先备份
# …插入配置段（token 从 /etc/imboy-textguard/check 读，不要手抄）…
erl -noshell -eval "case file:consult(\"$CFG\") of {ok,_} -> io:format(\"PARSE_OK~n\"); E -> io:format(\"FAIL:~p~n\",[E]) end" -s init stop
```

蓝绿发布产生的新 release 目录**不会自动继承**这段配置——每次发布要么在部署配置源里
带上，要么发布后重走 §6.3 + §6.4。

### 6.5 回退（分三级，从快到慢）

1. **秒级热回退**（业务立即恢复旁路）：
   `printf 'moderation_shadow:set_mode(legacy).\n' | $ERL -s -name $NODE -c "$COOKIE" -e`
2. **持久层回退**：把 sys.config 里该段的 `mode` 改为 `legacy`（或整段删除），再跑一次
   `file:consult` 校验；下次重启即以 legacy 启动。
3. **彻底下线**：`systemctl stop imboy-textguard && systemctl disable imboy-textguard`。
   后端 fail-open 下业务不受影响，仅失去检测。

### 6.6 已知坑（实操踩过的）

- `erl_call` 的 cookie 参数是 **`-c`**，不是 `-cookie`。
- `erl_call -a` 的参数解析器**不支持 binary 字面量**，带 `<<"...">>` 的调用必须用
  `-e` 从 stdin 喂表达式，且**表达式末尾要跟换行**。
- erl_call 探针里写 `<<"中文"/utf8>>` 会被节点的 latin1 解析器**二次编码**成乱码，
  导致"探针测不中"的假象；真实 App 路径（Cowboy 解析 JSON）无此问题。探测请用
  §6.3 的字节精确写法，或直接用 [verify-production.md](verify-production.md) 的 curl。
- shadow 观察期 `status()` 里 `version_incomparable` 计数是**预期现象**（后端内置词表
  与 TextGuard 词库版本天然不同，无法逐词对比），不是故障。

## 7. 常见故障

| 现象 | 原因与处理 |
|---|---|
| 服务反复重启，日志 `SECRET_FILE_UNREADABLE` | secret 文件属主不是 `imboy-tg`，`chown imboy-tg` 两个 secret 文件 |
| ready 一直是 503 | 尚未发布规则包，走 §4 |
| 发布 400 INVALID_RULESET | 空包缺 `x-textguard-confirm-empty: true`，或 JSON 字段/取值非法 |
| 发布 503 UNSUPPORTED_RULESET | 规则包 `t2s_table_id`/`normalization_id` 与服务启动值不一致 |
| 发布 412 | If-Match 不是当前 etag，重新 GET 取值（同内容幂等 200 属正常） |
| 切流后所有检测都 allow | TextGuard 停了 / `token` 或 `http_base_url` 配错 → fail-open 放行；查 `systemctl is-active imboy-textguard` 与 §6.2 配置 |
| `status` 显示 legacy 但已做过热切换 | 节点重启过且 sys.config 没有该段（热切换只在内存）；重走 §6.3 + §6.4 |
