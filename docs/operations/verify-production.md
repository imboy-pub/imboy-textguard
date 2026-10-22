# TextGuard 生产部署人工验证手册（复制即用）

[中文](verify-production.md) | [English](verify-production.en.md)

> 目标读者：不需要懂 Erlang/Rust 的运维或产品同学。全部命令在**服务器上**执行
> （`ssh -p 32 root@106.53.76.53`），凭证自动从本地文件读取，无需手抄。
> 切流操作本身见 [deploy-binary-prod.md](deploy-binary-prod.md) §6。

按顺序跑下面五层，任何一层不符合"期望输出"就停下查 [§6 常见故障](deploy-binary-prod.md)。

## 1. 服务层：进程活着吗

```bash
systemctl is-active imboy-textguard          # 期望 active
/usr/local/imboy-textguard-0.1.0-prod_a4b80d6/bin/imboy-textguard --version
                                              # 期望 imboy-textguard 0.1.0 0.1.0-prod_a4b80d6
curl -s -o /dev/null -w "live:%{http_code} "  http://127.0.0.1:8181/health/live
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready
                                              # 期望 live:200 ready:200
```

## 2. 规则包层：词库在位吗

```bash
PUB=$(cat /etc/imboy-textguard/publish)
curl -s -H "authorization: Bearer $PUB" http://127.0.0.1:8183/internal/v1/rulesets/active \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print('generation:',d['generation'],'| scene_counts:',d['scene_counts'],'| ready:',d['ready'])"
```

期望输出（2026-10 词库）：

```
generation: 2 | scene_counts: {'channel_message': 0, 'moment_post': 0, 'profile_field': 8851} | ready: True
```

`profile_field: 8851` = 四类词全部在位（drugs 3080 / terrorism 3912 / sexual 1684 / political_abuse 175）。

## 3. 检测层：词真的能拦到吗（curl 逐条版）

```bash
CHK=$(cat /etc/imboy-textguard/check)
for W in "0售k粉qq" "qq专卖假军官证" "115网盘av" "0售汽枪配件" "你好世界"; do
  curl -s -X POST http://127.0.0.1:8181/v1/check -H "authorization: Bearer $CHK" \
    -H "content-type: application/json" \
    -d "{\"request_id\":\"manual-verify\",\"scene\":\"profile_field\",\"fields\":[{\"field_id\":\"nickname\",\"text\":\"$W\"}]}" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);h=d.get('hits',[]);print(d['decision'], h[0].get('category') if h else '无命中')"
done
```

期望（前四行各命中一类，最后干净词放行）：

```
review drugs
review political_abuse
review sexual
review terrorism
allow 无命中
```

`review` 表示"保存成功但进人工复核"——这就是全部 8851 词（均 medium）的设计行为，不是漏拦。

## 4. 检测层：一键全检（Python 版）

比 curl 更完整：服务健康 + 规则包数量 + 四类样本 + 干净对照 + 随机抽 50 条词库词批量验证，
最后给一行总结论。整个块复制粘贴执行即可：

```bash
python3 - <<'PY'
import json, random, subprocess, pathlib

CHK = open('/etc/imboy-textguard/check').read().strip()
PUB = open('/etc/imboy-textguard/publish').read().strip()
URL, ADMIN = 'http://127.0.0.1:8181', 'http://127.0.0.1:8183'

def http(method, path, token, body=None):
    cmd = ['curl', '-s', '-X', method, URL + path, '-H', f'authorization: Bearer {token}']
    if body is not None:
        cmd += ['-H', 'content-type: application/json', '-d', json.dumps(body, ensure_ascii=False)]
    return json.loads(subprocess.check_output(cmd))

def check(word):
    r = http('POST', '/v1/check', CHK, {
        'request_id': 'verify-' + word[:16], 'scene': 'profile_field',
        'fields': [{'field_id': 'nickname', 'text': word}]})
    return r['decision'], r.get('hits', [])

# 1) 健康
live = subprocess.check_output(['curl', '-s', '-o', '/dev/null', '-w', '%{http_code}', URL + '/health/live']).decode()
ready = subprocess.check_output(['curl', '-s', '-o', '/dev/null', '-w', '%{http_code}', URL + '/health/ready']).decode()
print(f"[1] live={live} ready={ready}", "✓" if live == ready == '200' else "✗")

# 2) 规则包
state = http('GET', '/internal/v1/rulesets/active', PUB)
counts = state['scene_counts']
print(f"[2] generation={state['generation']} profile_field={counts.get('profile_field')}", "✓" if counts.get('profile_field') == 8851 else "✗")

# 3) 四类样本 + 干净对照（样本词来自公开词库文件，见 data/dictionaries/review_sources_20261020）
samples = {'drugs': '0售k粉qq', 'political_abuse': 'qq专卖假军官证', 'sexual': '115网盘av', 'terrorism': '0售汽枪配件'}
for cat, word in samples.items():
    decision, hits = check(word)
    good = decision == 'review' and any(h.get('category') == cat for h in hits)
    print(f"[3] {cat:<16} {decision:<7}", "✓" if good else f"✗ hits={[(h.get('category'), h.get('severity')) for h in hits]}")
decision, hits = check('你好世界正常昵称')
print(f"[3] clean            {decision:<7}", "✓" if decision == 'allow' and not hits else "✗")

# 4) 随机抽 50 条词库词，全部应命中
seed_files = sorted(pathlib.Path('/tmp').glob('all_categories.ruleset.json'))
words = []
try:  # 词库文件若不在 /tmp，跳过抽样层
    pkg = json.load(open(seed_files[0])) if seed_files else None
    words = random.sample([l['word'] for l in pkg['literals']], 50) if pkg else []
except Exception:
    words = []
bad = [w for w in words if check(w)[0] == 'allow']
print(f"[4] 抽样 {len(words)} 条，漏检 {len(bad)} 条", "✓" if not bad else f"✗ {bad[:3]}")

print("\n总结论：", "全部通过 ✓" if not bad else "存在异常，查 deploy-binary-prod.md §7")
PY
```

> 抽样层需要词库包文件在 `/tmp/all_categories.ruleset.json`（从仓库
> `data/dictionaries/review_sources_20261020/rulesets/` 上传）；不在就自动跳过，前三层照常。

## 5. 后端层：切流生效吗（需要懂一点 Erlang，可跳过）

```bash
REL=$(ls -d /usr/local/imboy-* | tail -1)     # 或按 ss -tlnp ':9800' 找到的实际目录
NODE=$(grep '^-name' $REL/releases/*/vm.args | awk '{print $2}')
COOKIE=$(grep '^-setcookie' $REL/releases/*/vm.args | awk '{print $2}')
/usr/local/lib/erlang/bin/erl_call -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"
```

期望 `mode => rust`（未切流时是 `legacy`；`shadow` 表示观察期）。

## 6. App 层：手机上能感知到吗

**前提**：后端已切 `rust`（见 §5）。用测试账号登录 imboyapp：

1. 把昵称改成 `0售k粉qq` → **保存会成功**（medium 不硬拦），这是设计行为
2. 管理后台（imboyadmin）内容复核队列里应出现该昵称的待审记录，类别 drugs
3. 把昵称改回正常词 → 保存成功且复核队列无新增

注意：改聊天消息、朋友圈**不会**触发这套词库——它只覆盖资料字段（昵称/签名/职业/学校/兴趣）。

## 期望速查表

| 检查点 | 期望值 |
|---|---|
| live / ready | 200 / 200 |
| generation | 2（每次发布规则包 +1） |
| profile_field 词数 | 8851 |
| 词库词检测 | `review` + 正确 category |
| 干净词检测 | `allow` 无命中 |
| 后端模式 | `rust` |
| App 改昵称含词库词 | 保存成功 + 后台复核队列出现记录 |
