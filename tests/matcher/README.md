# T08 字面量/别名组件

[中文](README.md) | [English](README.en.md)

使用隔离的 `CARGO_TARGET_DIR` 运行 `cargo test --locked --offline --test matcher -- --nocapture`。
发现（列出）用同一命令加 `-- --list`。驱动器使用导出的库匹配器（25 项测试）。T08 还注册了
库级聚合与两项扩展预算检查（3 项测试），选择方式为
`cargo test --locked --lib matcher:: -- --skip matcher::regex::`。

差分测试读取冻结的 T05 语料，校验其 manifest SHA256 与 seed 20261016，并把全部 10,000 个用例
与真实匹配器逐一比对。它只从冻结期望中投影出字面量/别名见证。gap 见证被计为推迟到 T09/T11；
既不伪造，也不标记通过。正则与最终请求聚合属于 T10/T11。`oracle-ruleset.json` 是
`tests/oracle/rulepool.py` 中合成数据的精确 JSON 快照；期望命中永不来自生产代码。

`CompiledLiterals::compile` 返回静态的、不含内容的 `CompileError::issue`；
`ErrorCode::from(error)` 是发布适配器。Compat B、enhanced B 与 enhanced T 的 canonical 模式
使用对应的真实归一化器。别名仅用于 enhanced B。一个逻辑 canonical 贡献三条编译条目，
一个别名贡献一条；每条元数据条目及其归一化字节在归一化文本去重之前计数。扇出按自动机
分别检查。

`scan_text(text, scene, enable_t2s, budget, cancel)` 是组件便捷 API。`scan_base` 与 `scan_t2s`
接受供 T11 流水线使用的来源单元。同一个 `CandidateBudget` 必须贯穿整个请求：每个字段、
B/T/G、元数据扩展与例外出现。已匹配的模式按每条元数据条目消耗一个候选，且发生在场景或
边界过滤之前；原始工作与扩展工作不重复计费。G/例外消费者必须使用同一 `consume` API。
最多 20,000 个候选成功；消耗第 20,001 个即返回错误。候选预算失败以及通过预算观察到的
取消保持终态。被中断的扫描不存在部分命中的成功。请求级响应排序、字段 ID、元数据富化、
例外抑制与最终 100 命中上限仍属 T11 义务。

组件测试包含 OTP 兼容的 `ΟΣ -> οσ` 行为与增强的整串 `ΟΣ -> ος` 行为。它们不改变 T06 的旧向量。
