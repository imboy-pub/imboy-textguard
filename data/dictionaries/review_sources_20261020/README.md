# 三轮来源词库补充：资料字段复审候选

这是可复现的来源词条数据集，不是经人工业务验证的生产封禁词库。确切数量、三轮增量和距每类 10,000 的缺口见 `COUNTS.json`；不足不拼接、不补造、不重复计数。

## 数据与三轮来源

先依据固定源文件重建上一版 1,428 条的成员摘要，再剔除不适合此次发布的年龄/身份/隐私等项。第1轮补齐 kirklin/go-swd 完整文件及既有 konsheng 小文件；第2轮读取 houbb/sensitive-word-data 完整 tagged 文件；第3轮读取 konsheng 的 Tencent、网易、补充、涉枪涉爆完整文件。源仓可能相互转载，不把多来源等同于独立事实确认。

所有来源锁定 commit 和 Git blob，下载全文验证后才解析。逐词保留源文件ID、原始行号、原词和源标签；`provenance/sources.json` 提供路径、许可、版本及 SHA-256。源标签仅供追溯，不能把政治人物、宗教或普通政治话题自动当作违规。词条在开源源文件中存在不代表它是真实犯罪事实、有效药物或已获得业务批准。上游自己可能存在组合词和过时黑话，本包不为凑数自行扩展。

`terrorism` 沿用项目分类键，其中区分恐怖主义主题、暴力威胁、武器/爆炸物推广；并非每个词均指恐怖主义。`political_abuse` 限于官方身份假冒、假证和相关滥用，不是政治立场或人物黑名单。药物名称/营销词可能存在医疗、教育、新闻语境。

## 文件

- `words/`：四类每行一词。
- `rulesets/`：四个单类完整包及合并完整包，均符合本仓 Schema。
- `provenance/`：逐词来源、完整源文件摘要和旧基线剔除记录。
- `licenses/`：各源仓原许可全文。第三方数据不改成项目 Mulan 许可。
- `COUNTS.json`：实收数量、每轮新增与缺口。

## 行为与导入边界

本 PR **不改变** `default_review_seed.json`、首次启动逻辑或任何已生效 ACTIVE。新增文件不会因重启自动生效；不要删除 ACTIVE 强制重跑初始化。

新包仅有 `profile_field`，`medium`（review），`compat-v1`，无别名/gap/regex/例外。名称含某词只产生复审线索，不能直接认定违法；review 不等于业务已经阻止保存。当前 API 按场景而非 field_id 限定，profile 简介也会受同场景词条影响。

管理 API 是**全量快照替换**。不要依次 PUT 四个单类包，不要用 profile-only 包直接覆盖原有频道/动态规则。应在隔离环境验证后，先从权威真源导出完整规则，按归一化匹配键去重，保留旧词条ID、风险动作、场景、正则及例外，再完整校验并经 PUBLISH + If-Match 发布。IMBoy PG 发布者仍是真源时，必须先导入真源，否则周期对账会覆盖手工发布。

本版生成ID使用独立 `tg-review-v2` 摘要命名空间，与旧下载包ID不保证相同，也不是数据库记录ID。合并不得按新ID盲目追加。默认52条保持不变。

## 重现与验证

```sh
# 只在维护/构建时访问固定来源；生产运行不联网下载词库。
python3 tools/lexicon/build_review_lexicon.py --fetch --cache /tmp/textguard-source-cache
python3 -m unittest discover -s tools/lexicon -p 'test_*.py' -v
python3 tools/lexicon/validate_review_lexicon.py
cargo test --locked --test review_lexicon
```

静态 Schema 校验需要 `jsonschema==4.23.0`。Rust 测试读取本地文件并编译真实 Snapshot，逐词检查 review/场景隔离，以及全量发布、重启恢复和旧 bootstrap 未被替换。测试不是业务准确率评估；没有代表性标注集时不宣传零误报。本数据包保持可选，启用和动作变更由业务负责人决定。
