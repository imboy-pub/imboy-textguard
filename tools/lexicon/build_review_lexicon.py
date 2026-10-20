#!/usr/bin/env python3
"""Reproduce three locked, source-observed collection rounds; never publish rules.
Raw source files are untrusted DATA, never executed or included in the distribution.
Python 3.10+; --fetch requires network. Offline validation uses checked-in output.
"""
from __future__ import annotations
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import sys
import time
import urllib.parse
import urllib.request

CATEGORIES = ('sexual', 'drugs', 'terrorism', 'political_abuse')
SCENES = ('channel_message', 'moment_post', 'profile_field')
ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / 'data/dictionaries/review_sources_20261020'
MAX_SOURCE = 4 * 1024 * 1024


def norm(text: str) -> str:
    return ''.join(chr(ord(c)-0xfee0) if 0xff01 <= ord(c) <= 0xff5e else c
                   for c in text.lower() if c not in '\u200b\u200c\u200d\ufeff')


def blob(data: bytes) -> str:
    return hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def dump(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def fetch(source: dict, cache: Path, online: bool) -> bytes:
    """All inputs are pinned by a 40-character commit and expected Git blob."""
    for field in ('ref', 'blob'):
        if not re.fullmatch('[a-f0-9]{40}', source[field]):
            raise ValueError('Unpinned source')
    path = cache / source['blob']
    if path.exists():
        data = path.read_bytes()
    elif online:
        url = 'https://raw.githubusercontent.com/' + source['repo'] + '/' + source['ref'] + '/' + urllib.parse.quote(source['path'], safe='/')
        last = None
        for attempt in range(3):
            try:
                req = urllib.request.Request(url, headers={'User-Agent': 'TextGuard-lexicon-audit/1'})
                with urllib.request.urlopen(req, timeout=30) as response:
                    data = response.read(MAX_SOURCE + 1)
                break
            except (OSError, TimeoutError) as error:
                last = error
                time.sleep(attempt + 1)
        else:
            raise RuntimeError('Source fetch failed: ' + source.get('id', source['repo'])) from last
        cache.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    else:
        raise RuntimeError('Missing pinned cache: ' + source.get('id', source['repo']))
    if len(data) > MAX_SOURCE or blob(data) != source['blob']:
        raise ValueError('Source size/blob mismatch: ' + source.get('id', source['repo']))
    data.decode('utf-8-sig')
    return data


def records(data: bytes, kind: str):
    for line_no, line in enumerate(data.decode('utf-8-sig').splitlines(), 1):
        line = line.strip()
        if not line or line.startswith(('#', '//')):
            continue
        tags = []
        if kind == 'tags':
            match = re.fullmatch(r'(.+)\s+([0-9]+(?:,[0-9]+)*)', line)
            if not match:
                raise ValueError('Malformed tagged source at line ' + str(line_no))
            line, raw = match.groups()
            tags = raw.split(',')
        # A whole source line is one observed term. No splitting/prefix expansion.
        yield line_no, line, tags


# New candidates must be short observable signals, not graphic narratives,
# contact endpoints, names/political positions, or copied passages.
EXCLUDE = re.compile(r'幼|兒童|儿童|女童|男童|未成年|小孩|萝莉|蘿莉|loli|童颜|童顏|小学生|中学生|初中|高中|少女|少年|学生|學生|校花|儿子|女儿|妹妹|弟弟|妈妈|母亲|姐姐|阿姨|嫂|亂倫|乱伦|兽交|人兽|母子|父女|强奸|強奸|轮奸|迷奸|亂倫|[0-9]{1,2}[岁歲嵗]|[0-9]{5,}|https?[:/]|www[.点]|\.(?:com|net|cn|org)|[点點](?:com|net|cn)|微信号|加微|加v|威信|联系我|联系qq|加q|\bqq[0-9]|习近平|習近平|毛泽东|江泽民|胡锦涛|共产党|共產黨|法轮|法輪|达赖|達賴|藏独|疆独', re.I)
SEX = re.compile(r'色情|淫秽|淫穢|招嫖|卖淫|賣淫|嫖娼|裸聊|性交易|性服务|性服務|成人(?:影院|电影|電影|影片|视频|視頻|小说|小說|网站|網站|论坛|論壇|直播)|黄色(?:电影|電影|影片|网站|網站|视频|視頻)|情色(?:电影|電影|影片|视频|視頻|网站|網站)|av(?:影片|视频|視頻|电影|電影|影院|资源|資源)|三级片|三點片|三级片|三级(?:片|影片)|黄色片')
DRUG = re.compile(r'冰毒|海洛因|可卡因|摇头丸|搖頭丸|甲基苯丙胺|甲基安非他|安非他命|苯丙胺|氯胺酮|氯氨酮|卡西酮|喵喵药|麻古|麻果|麻谷|缅古|大麻|鸦片|鴉片|罂粟|罌粟|致幻蘑菇|迷幻蘑菇|神奇蘑菇|麦角酸|毒品|k粉|ghb|lsd|mdma', re.I)
SEDATIVE = re.compile(r'迷药|迷藥|迷魂|蒙汗药|蒙汗藥|乖乖水|乖乖药|听话水|听话药|失忆粉|失忆药|拍肩药|拍肩水|迷香药|迷幻喷雾|迷幻香烟')
MEDICINE = re.compile(r'可待因|吗啡|嗎啡|芬太尼|埃托啡|唑仑|唑侖|杜冷丁|度冷丁|美沙酮|羟考酮|曲马多|氟硝西泮|氢可酮|氢吗啡|吡咯戊酮|赛洛新|蒂巴因|尼可吗啡')
TRADE = re.compile(r'出售|售卖|售賣|贩卖|販賣|买卖|買賣|专卖|專賣|批发|批發|供应|供應|订购|訂購|购买|購買|销售|銷售|直销|直銷|提供|供货|供貨|送货|送貨|到货|出货|转让|轉讓|交易|货到付款|卖|賣')
WEAPON = re.compile(r'手枪|手槍|步枪|步槍|冲锋枪|衝鋒槍|狙击枪|狙擊槍|枪支|槍支|枪械|槍械|气枪|氣槍|汽枪|汽槍|炸弹|炸彈|炸药|炸藥|雷管|手榴弹|手榴彈|子弹|子彈|爆炸物|燃烧弹|燃燒彈|火药|火藥|c4|tnt|petn', re.I)
TERROR = re.compile(r'恐怖(?:组织|組織|活动|活動|袭击|襲擊|分子|份子)|自杀式(?:袭击|襲擊)|人体炸弹|人肉炸弹')
VIOLENT = re.compile(r'雇佣杀手|雇傭殺手|雇职业杀手|雇杀手|杀手雇佣|雇凶|雇兇|专业(?:帮人|替人|代人)(?:杀人|砍人|绑架)|专业提供杀手|杀你全家|杀你1家|捅死你|威胁杀害|威胁袭击|暴力恐吓')
OFFICIAL = re.compile(r'警官证|警官證|军官证|軍官證|军警|軍警|警服|警徽|警衔|警銜|警号|警號|政府|公检法|公檢法|公安|公职|公職|选举|選舉|投票')
FAKE = re.compile(r'假|伪造|偽造|仿真|高仿|冒充|诈骗|詐騙|恐吓|恐嚇|威胁|威脅|贿选|賄選')
BARE = {'色情', '情色', '成人', '警服', '政府', '公安', '投票', '恐怖', '麻醉药', '咖啡因'}


def constraint(word: str, *, baseline: bool = False) -> str | None:
    if not (2 <= len(word) <= (64 if baseline else 24)) or len(word.encode()) > 192:
        return 'length'
    if any(ord(c) < 32 for c in word) or any(c in word for c in ',，\t'):
        return 'separator_or_control'
    if not re.search(r'[\u3400-\u9fff]', word):
        return 'not_chinese'
    if EXCLUDE.search(word):
        return 'safety_privacy_or_identity_exclusion'
    if word in BARE:
        return 'overbroad'
    return None


def classify(word: str) -> tuple[str, str] | None:
    if OFFICIAL.search(word) and FAKE.search(word):
        return 'political_abuse', 'official_impersonation_or_forgery_context_required'
    if DRUG.search(word):
        return 'drugs', 'drug_topic_or_trade_context_required'
    if SEDATIVE.search(word):
        return 'drugs', 'unverified_incapacitant_marketing_not_efficacy_claim'
    if MEDICINE.search(word) and TRADE.search(word):
        return 'drugs', 'medication_trade_context_required_not_illegality_claim'
    if TERROR.search(word):
        return 'terrorism', 'terrorism_topic_context_required'
    if VIOLENT.search(word):
        return 'terrorism', 'violence_threat_or_contract_killing_not_terrorism_finding'
    if WEAPON.search(word) and (TRADE.search(word) or re.search(r'制作|製作|自制|制造|製造|配方|图纸|圖紙|教程', word)):
        return 'terrorism', 'weapon_or_explosive_promotion_not_terrorism_finding'
    if SEX.search(word):
        return 'sexual', 'adult_content_or_solicitation_context_required'
    return None


def parse_ranges(text: str):
    for item in text.split(','):
        parts = [int(x) for x in item.split('-')]
        yield from range(parts[0], parts[-1] + 1)


def make_pack(terms: list[dict]) -> dict:
    return dict(schema_version=1, normalization_id='imboy-base-v1',
                t2s_table_id='opencc-1.1.9-single-v1', limit_profile_id='mvp-default-v1',
                semantics_by_scene={s: 'compat-v1' for s in SCENES}, allow_empty=False,
                literals=[dict(id=t['id'], word=t['word'], category=t['category'],
                               severity='medium', scenes=['profile_field'], aliases=[],
                               gap=dict(enabled=False, action='review')) for t in terms],
                regex_rules=[], exceptions=[])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--fetch', action='store_true')
    ap.add_argument('--cache', type=Path, required=True)
    ap.add_argument('--out', type=Path, default=DATA)
    args = ap.parse_args()
    lock = json.loads((Path(__file__).with_name('sources.lock.json')).read_text())
    fetched, source_meta = {}, []
    for s in lock['sources']:
        data = fetch(s, args.cache, args.fetch)
        recs = list(records(data, s['format']))
        fetched[s['id']] = (s, recs)
        source_meta.append({**s, 'sha256': sha(data), 'bytes': len(data),
                            'records': len(recs), 'coverage': 'complete', 'blob_verified': True})
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    for lic in lock['licenses']:
        payload = fetch(lic, args.cache, args.fetch)
        (out / 'licenses').mkdir(exist_ok=True)
        (out / 'licenses' / lic['destination']).write_bytes(payload)
    baseline = {}
    for selection in lock['baseline']['selections']:
        s, recs = fetched[selection['source']]
        by_line = {line: (word, tags) for line, word, tags in recs}
        by_word = {word: (line, tags) for line, word, tags in recs}
        chosen = []
        for line in parse_ranges(selection['lines']) if 'lines' in selection else []:
            word, tags = by_line[line]; chosen.append((line, word, tags))
        for word in selection.get('words', []):
            line, tags = by_word[word]; chosen.append((line, word, tags))
        for line, word, tags in chosen:
            key = (selection['category'], norm(word))
            baseline[key] = (s, line, word, tags)
    keys = ''.join(c + '\t' + w + '\n' for c, w in sorted(baseline))
    if len(baseline) != lock['baseline']['count'] or sha(keys.encode()) != lock['baseline']['normalized_keys_sha256']:
        raise ValueError('Previous 1,428-term membership could not be reconstructed exactly')
    terms = {}
    excluded = collections.Counter()
    audit = []
    def add(s, line, word, tags, category, subcategory, round_id, is_base=False):
        reason = constraint(word, baseline=is_base)
        if reason:
            excluded[reason] += 1
            if is_base:
                audit.append({'category': category, 'word_sha256': sha(word.encode()), 'reason': reason})
            return False
        key = norm(word)
        origin = {'source_id': s['id'], 'line': line, 'word': word, 'source_tags': tags}
        if key in terms:
            if origin not in terms[key]['sources']:
                terms[key]['sources'].append(origin)
            return False
        ident = str(int.from_bytes(hashlib.sha256(('tg-review-v2\0' + category + '\0' + key).encode()).digest()[:8], 'big') & ((1 << 63) - 1))
        if ident == '0':
            raise ValueError('Zero generated identifier')
        terms[key] = dict(id=ident, word=word, category=category, subcategory=subcategory,
                          normalized=key, first_round=round_id, sources=[origin],
                          business_validation='NOT_VALIDATED', synthetic_expansion=False)
        return True
    for (category, key), (s, line, word, tags) in sorted(baseline.items()):
        sub = classify(word)
        add(s, line, word, tags, category, sub[1] if sub and sub[0] == category else 'previous_source_screened_candidate_context_required', 0, True)
    rounds = []
    baseline_kept = len(terms)
    for rnd in (1, 2, 3):
        before = collections.Counter(t['category'] for t in terms.values())
        scanned = 0
        for source_id, (s, recs) in fetched.items():
            if s['round'] != rnd:
                continue
            for line, word, tags in recs:
                scanned += 1
                reason = constraint(word)
                if reason:
                    excluded[reason] += 1; continue
                selected = classify(norm(word))
                if selected is None:
                    excluded['no_selected_risk_signal'] += 1; continue
                category, subcategory = selected
                add(s, line, word, tags, category, subcategory, rnd)
        after = collections.Counter(t['category'] for t in terms.values())
        rounds.append({'round': rnd, 'source_ids': [s['id'] for s, _ in fetched.values() if s['round'] == rnd],
                       'observed_records': scanned,
                       'new_unique': {c: after[c] - before[c] for c in CATEGORIES},
                       'cumulative': {c: after[c] for c in CATEGORIES}})
    ordered = sorted(terms.values(), key=lambda t: (t['category'], t['normalized']))
    if len(ordered) > 50000 or len({t['id'] for t in ordered}) != len(ordered):
        raise ValueError('Total rule budget or ID collision')
    for t in ordered:
        t['sources'].sort(key=lambda s: (s['source_id'], s['line']))
    for c in CATEGORIES:
        selected = [t for t in ordered if t['category'] == c]
        if not selected:
            raise ValueError('Empty category: ' + c)
        (out / 'words').mkdir(exist_ok=True)
        (out / 'words' / (c + '.txt')).write_text(''.join(t['word'] + '\n' for t in selected), encoding='utf-8')
        dump(out / 'rulesets' / (c + '.ruleset.json'), make_pack(selected))
    dump(out / 'rulesets/all_categories.ruleset.json', make_pack(ordered))
    (out / 'provenance').mkdir(exist_ok=True)
    (out / 'provenance/terms.jsonl').write_text(''.join(json.dumps(t, ensure_ascii=False, separators=(',', ':')) + '\n' for t in ordered), encoding='utf-8')
    dump(out / 'provenance/sources.json', source_meta)
    dump(out / 'provenance/baseline_exclusions.json', audit)
    counts = dict(collections.Counter(t['category'] for t in ordered))
    summary = {'baseline_reconstructed': len(baseline), 'baseline_retained': baseline_kept,
               'baseline_safety_excluded': len(audit), 'rounds': rounds, 'counts': counts,
               'total': len(ordered), 'target_each': 10000,
               'shortfall': {c: max(0, 10000-counts[c]) for c in CATEGORIES},
               'excluded_record_counts_not_unique_terms': dict(excluded),
               'generated_terms': 0, 'business_accuracy': 'NOT_MEASURED',
               'id_policy': 'tg-review-v2 hash; new distribution namespace, not IMBoy database IDs',
               'default_bootstrap_changed': False, 'active_rules_changed': False}
    dump(out / 'COUNTS.json', summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, RuntimeError) as exc:
        print('ERROR: ' + str(exc), file=sys.stderr)
        sys.exit(1)
