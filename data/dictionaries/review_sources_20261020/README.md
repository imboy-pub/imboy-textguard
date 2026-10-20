# 三轮来源词库补充：8,851 条资料字段复审候选

这是可复现的来源词条数据集，不是经人工业务验证的生产封禁词库。未达到每类10,000，不拼接、不补造、不重复计数。确切数量和缺口可由 `COUNTS.json` 重现。

| 类别 | 最终条数 | 距离10,000 |
|---|---:|---:|
| sexual | 1,684 | 8,316 |
| drugs | 3,080 | 6,920 |
| terrorism（含暴力/武器风险） | 3,912 | 6,088 |
| political_abuse（官方身份滥用） | 175 | 9,825 |
| 合计 | 8,851 | 31,149 |

## 数据与三轮来源

先依据固定源文件重建上一版1,428条的成员摘要，再剔除14条不适合此次发布的条目，保留1,414条。

| 轮次 | 完整读取的来源 | 净新增去重词条 |
|---|---|---:|
| 第1轮 | kirklin/go-swd 完整分类文件与既有konsheng小文件 | 918 |
| 第2轮 | houbb/sensitive-word-data 完整tagged文件 | 6,184 |
| 第3轮 | konsheng的Tencent、网易、补充、涉枪涉爆完整文件 | 335 |

14个完整源文件共读取119,406条记录。源仓可能相互转载，不把多来源等同于独立事实确认。所有来源锁定commit和Git blob，下载全文验证后才解析。逐词保留源文件ID、原始行号、原词和源标签；`provenance/sources.json` 提供路径、许可、版本及SHA-256。

源标签仅供追溯，不能把政治人物、宗教或普通政治话题自动当作违规。词条在开源源文件中存在不代表它是真实犯罪事实、有效药物或已获得业务批准。上游自己可能存在组合词和过时黑话，本包不为凑数自行扩展。没有逐条人工语义审核或代表性业务误报率证明。

`terrorism` 沿用项目分类键，其中区分恐怖主义主题、暴力威胁、武器/爆炸物推广；并非每个词均指恐怖主义。`political_abuse` 限于官方身份假冒、假证和相关滥用，不是政治立场或人物黑名单。药物名称/营销词可能存在医疗、教育、新闻语境。

## 文件

- `words/`：四类每行一词。
- `rulesets/`：四个单类完整包及合并完整包，均符合本仓Schema。
- `provenance/`：逐词来源、完整源文件摘要和旧基线剔除记录。
- `licenses/`：各源仓原许可全文。第三方数据不改成项目Mulan许可。
- `COUNTS.json`：实收数量、每轮新增与缺口。

## 行为与导入边界

本PR**不替换**内置52条 `default_review_seed.json`，不改动任何已生效ACTIVE。新增数据文件不会因重启自动生效；不要删除ACTIVE强制重跑初始化。

另修复PR #1的启动表版本问题：默认繁简表仍按原行为初始化52条；显式选择其他受支持表时，不用不兼容的默认包导致进程退出，而是保持live、NotReady，等待有权限的发布者提供兼容规则。损坏快照仍不回退到默认包。这不是放宽规则版本校验。

新包仅有 `profile_field`，`medium`（review），`compat-v1`，无别名/gap/regex/例外。名称含某词只产生复审线索，不能直接认定违法；review不等于业务已经阻止保存。当前API按场景而非field_id限定，profile简介也会受同场景词条影响。

管理API是**全量快照替换**。不要依次PUT四个单类包，不要用profile-only包直接覆盖原有频道/动态规则。应在隔离环境验证后，先从权威真源导出完整规则，按归一化匹配键去重，保留旧词条ID、风险动作、场景、正则及例外，再完整校验并经PUBLISH + If-Match发布。IMBoy PG发布者仍是真源时，必须先导入真源，否则周期对账会覆盖手工发布。

本版生成ID使用独立 `tg-review-v2` 摘要命名空间，与旧下载包ID不保证相同，也不是数据库记录ID。合并不得按新ID盲目追加。默认52条保持不变。

## 重现与验证

```sh
# 只在维护/构建时访问固定来源；生产运行不联网下载词库。
python3 tools/lexicon/build_review_lexicon.py --fetch --cache /tmp/textguard-source-cache
python3 -m unittest discover -s tools/lexicon -p 'test_*.py' -v
python3 tools/lexicon/validate_review_lexicon.py
cargo test --locked --test review_lexicon --test bootstrap_table
cargo test --locked --features fault-injection --test fault_publish --test fault_process
```

静态Schema校验需要 `jsonschema==4.23.0`。Rust测试读取本地文件并编译真实Snapshot，逐词检查review/场景隔离，以及全量发布、重启恢复和旧bootstrap未被替换。数据集生成与校验不需要Python生产运行时。常规CI不联网采集词库，没有自动推送或自动更新生效规则的工作流。

测试不是业务准确率评估；没有代表性标注集时不宣传零误报。本数据包保持可选，启用和动作变更由业务负责人决定。
