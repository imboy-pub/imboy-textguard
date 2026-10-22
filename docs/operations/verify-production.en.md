# TextGuard Production Verification Handbook (copy-paste ready)

[中文](verify-production.md) | [English](verify-production.en.md)

> Audience: operators who do not need to know Erlang/Rust. Run everything **on the server**
> (`ssh -p 32 root@106.53.76.53`). Credentials are read from local files — never typed.
> For the switch-over itself see [deploy-binary-prod.en.md](deploy-binary-prod.en.md) §6.

Work through the five layers in order; on any mismatch stop and check the
troubleshooting table in [deploy-binary-prod.en.md](deploy-binary-prod.en.md) §7.

## 1. Service layer: is the process alive

```bash
systemctl is-active imboy-textguard          # expect active
/usr/local/imboy-textguard-0.1.0-prod_a4b80d6/bin/imboy-textguard --version
                                              # expect imboy-textguard 0.1.0 0.1.0-prod_a4b80d6
curl -s -o /dev/null -w "live:%{http_code} "  http://127.0.0.1:8181/health/live
curl -s -o /dev/null -w "ready:%{http_code}\n" http://127.0.0.1:8181/health/ready
                                              # expect live:200 ready:200
```

## 2. Ruleset layer: is the lexicon loaded

```bash
PUB=$(cat /etc/imboy-textguard/publish)
curl -s -H "authorization: Bearer $PUB" http://127.0.0.1:8183/internal/v1/rulesets/active \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print('generation:',d['generation'],'| scene_counts:',d['scene_counts'],'| ready:',d['ready'])"
```

Expected (2026-10 lexicon): `generation: 2 | scene_counts: {'channel_message': 0, 'moment_post': 0, 'profile_field': 8851} | ready: True`
— `profile_field: 8851` means all four categories are loaded (drugs 3080 / terrorism 3912 / sexual 1684 / political_abuse 175).

## 3. Detection layer: do words actually hit (curl loop)

```bash
CHK=$(cat /etc/imboy-textguard/check)
for W in "0售k粉qq" "qq专卖假军官证" "115网盘av" "0售汽枪配件" "你好世界"; do
  curl -s -X POST http://127.0.0.1:8181/v1/check -H "authorization: Bearer $CHK" \
    -H "content-type: application/json" \
    -d "{\"request_id\":\"manual-verify\",\"scene\":\"profile_field\",\"fields\":[{\"field_id\":\"nickname\",\"text\":\"$W\"}]}" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);h=d.get('hits',[]);print(d['decision'], h[0].get('category') if h else 'no-hit')"
done
```

Expected: `review drugs` / `review political_abuse` / `review sexual` / `review terrorism` / `allow no-hit`.
`review` = "saved but queued for human moderation" — the designed behavior for all 8851 medium-severity terms.

## 4. Detection layer: one-shot full check (Python)

Health + ruleset count + one sample per category + clean control + 50 random lexicon words,
ending with a one-line verdict. Paste the whole block:

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

live = subprocess.check_output(['curl', '-s', '-o', '/dev/null', '-w', '%{http_code}', URL + '/health/live']).decode()
ready = subprocess.check_output(['curl', '-s', '-o', '/dev/null', '-w', '%{http_code}', URL + '/health/ready']).decode()
print(f"[1] live={live} ready={ready}", "OK" if live == ready == '200' else "FAIL")

state = http('GET', '/internal/v1/rulesets/active', PUB)
counts = state['scene_counts']
print(f"[2] generation={state['generation']} profile_field={counts.get('profile_field')}",
      "OK" if counts.get('profile_field') == 8851 else "FAIL")

samples = {'drugs': '0售k粉qq', 'political_abuse': 'qq专卖假军官证',
           'sexual': '115网盘av', 'terrorism': '0售汽枪配件'}
for cat, word in samples.items():
    decision, hits = check(word)
    good = decision == 'review' and any(h.get('category') == cat for h in hits)
    print(f"[3] {cat:<16} {decision:<7}", "OK" if good else f"FAIL hits={hits}")
decision, hits = check('normal nickname text')
print(f"[3] {'clean':<16} {decision:<7}", "OK" if decision == 'allow' and not hits else "FAIL")

packs = sorted(pathlib.Path('/tmp').glob('all_categories.ruleset.json'))
try:
    words = random.sample([l['word'] for l in json.load(open(packs[0]))['literals']], 50) if packs else []
except Exception:
    words = []
bad = [w for w in words if check(w)[0] == 'allow']
print(f"[4] sampled {len(words)} lexicon words, missed {len(bad)}", "OK" if not bad else f"FAIL {bad[:3]}")

print("\nVerdict:", "ALL PASS" if not bad else "CHECK deploy-binary-prod.en.md section 7")
PY
```

> Layer 4 sampling needs the ruleset package at `/tmp/all_categories.ruleset.json`
> (upload from repo `data/dictionaries/review_sources_20261020/rulesets/`); it skips gracefully if absent.

## 5. Backend layer: is traffic switched (skip if not your area)

```bash
REL=$(ls -d /usr/local/imboy-* | tail -1)     # or the dir found via ss -tlnp ':9800'
NODE=$(grep '^-name' $REL/releases/*/vm.args | awk '{print $2}')
COOKIE=$(grep '^-setcookie' $REL/releases/*/vm.args | awk '{print $2}')
/usr/local/lib/erlang/bin/erl_call -s -name $NODE -c "$COOKIE" -a "moderation_shadow status []"
```

Expect `mode => rust` (`legacy` = not switched; `shadow` = observation period).

## 6. App layer: can a phone user notice

With the backend in `rust` mode, log into imboyapp with a test account:

1. Change the nickname to `0售k粉qq` → **the save succeeds** (medium severity never hard-blocks) — by design.
2. The admin console (imboyadmin) moderation review queue should show a pending entry, category drugs.
3. Change the nickname back to something normal → save succeeds, no new queue entry.

Note: chat messages and moments do **not** go through this lexicon — profile fields only
(nickname / sign / profession / school / interests).

## Expected-values cheat sheet

| Check | Expected |
|---|---|
| live / ready | 200 / 200 |
| generation | 2 (+1 per ruleset publication) |
| profile_field count | 8851 |
| Lexicon word check | `review` + correct category |
| Clean word check | `allow`, no hits |
| Backend mode | `rust` |
| App nickname with lexicon word | save succeeds + review-queue entry in admin |
