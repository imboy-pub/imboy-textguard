# TextGuard 交付操作手册（整合版，2026-10-20）
来源：R4-R9 冒烟实测命令整合；所有命令均已实际执行验证。无任何 secret。

## 0. 候选
- 服务：imboy-textguard 仓 `textguard/run-20261016T075111Z/integration`（49dfe3f）
- 消费端：imboy 仓 `textguard/run-20261016T075111Z/imboy-integration`（a2fe60cd）

## 1. 构建（服务仓根目录）
```bash
cd /Users/leeyi/project/imboy.pub/imboy-textguard
git checkout textguard/run-20261016T075111Z/integration
cargo build --locked --release            # ≈21s
cargo test --locked --test contract       # 7/7
cargo test --locked --test grpc_contract  # 15/15（真实 TCP）
cargo fmt --all -- --check && cargo clippy --locked --all-targets -- -D warnings
```

## 2. 启动（先备 0600 secret 与空规则目录）
```bash
SEC=/Users/leeyi/tg-secrets; mkdir -p $SEC/rules
python3 -c "import secrets,string;a=string.ascii_letters+string.digits;print(''.join(secrets.choice(a) for _ in range(48)))" > $SEC/check
python3 -c "import secrets,string;a=string.ascii_letters+string.digits;print(''.join(secrets.choice(a) for _ in range(48)))" > $SEC/publish
chmod 600 $SEC/check $SEC/publish
# 注意：数据目录祖先不可含符号链接（防 symlink 加固，macOS 的 /tmp 不行）
TEXTGUARD_T2S_TABLE_ID=opencc-1.1.9-single-v1 \
TEXTGUARD_DATA_DIR=$SEC/rules TEXTGUARD_CHECK_SECRET_FILE=$SEC/check \
TEXTGUARD_PUBLISH_SECRET_FILE=$SEC/publish ./target/release/imboy-textguard
# 端口：HTTP 8081 / gRPC 8082 / 管理 8083（均 loopback）
```
启动即验：`curl -s -o /dev/null -w %{http_code} http://127.0.0.1:8081/health/live` → 200；`/health/ready` 未发布时 503。

## 3. 发布规则包（CAS）
```bash
PUB=$(cat $SEC/publish)
ETAG=$(curl -s -H "authorization: Bearer $PUB" http://127.0.0.1:8083/internal/v1/rulesets/active | python3 -c "import sys,json;print(json.load(sys.stdin)['state_etag'])")
curl -s -X PUT http://127.0.0.1:8083/internal/v1/rulesets/active \
  -H "authorization: Bearer $PUB" -H "If-Match: $ETAG" -H 'content-type: application/json' \
  --data-binary @<规则包>.json     # t2s_table_id 必须与启动表一致，否则 UNSUPPORTED_RULESET
```
行为：陈旧 etag + 同内容 → 幂等 200；陈旧 etag + 不同内容 → 412 CAS_CONFLICT；非法包（如例外引用缺失规则）→ 400。

## 4. 检测
```bash
CHK=$(cat $SEC/check)
curl -s -X POST http://127.0.0.1:8081/v1/check -H "authorization: Bearer $CHK" \
  -H 'content-type: application/json' \
  -d '{"request_id":"demo-1","scene":"channel_message","fields":[{"field_id":"f1","text":"…"}]}'
# 未认证 → 401；坏体 → 400；未发布 → 503 NOT_READY（绝不 allow）
```

## 5. 回滚 = 用上一版规则包重发布（同 §3，body 换成旧包、If-Match 用当前 etag）
发布历史在数据目录 publication-history/；ACTIVE 损坏时重启 → ready 503 fail-closed（实测）。

## 6. 消费端（imboy 仓）
```bash
cd /Users/leeyi/project/imboy.pub/imboy
git checkout textguard/run-20261016T075111Z/imboy-integration
# TextGuard EUnit 证据：runs/run-20261016T075111Z/T27-eunit-attempt4（TextGuard 9/9 绿）
scripts/verify_textguard.sh   # 需按冻结 commands.json 配好证据环境，无环境时 BLOCKED/exit2（设计如此）
```

## 7. 边界
- 生产 T29 与公开投稿 U05 未授权，不可执行。
- 启用限制：无业务授权语料 → 新增 block 规则强制 review/shadow（G6 restricted_local）。
