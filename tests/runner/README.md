# T04 运行器

[中文](README.md) | [English](README.en.md)

运行 `./scripts/verify_textguard.sh --group runner --evidence <new-external-directory>`。
`--list` 只列出冻结数组；`--dry-run` 记录 NOT_RUN 并以退出码 2 结束。
`--task T04` 运行 runner-selftest、t04-build-contract 与 t04-migration。构建选择器需要集成后的
树（`tests/build_contract.py`，骨架通道），不可用时报告 BLOCKED；迁移选择器需要
TEXTGUARD_FORK_ROOT。运行器自测仅覆盖 AC-010/114；它从不调用全量 EUnit。

`tests/manifest.json` 映射选择器与 AC。`tests/runner/commands.json` 保存字面 argv、发现 argv、
cwd、环境白名单、超时与计数解析器。额外的 Rust 选择器可用 `count_format: rust` 并配合显式的
`--list` 发现命令。每次尝试都需要新的证据目录。

可分发的实现是 `scripts/command_receipt.py` 与 `scripts/checkpoint.py`。运行器与普通自测只加载
这些仓库内文件；它们不依赖任何个人目录。其摘要与解析出的可执行文件在每次结果中留下指纹。
不存在隐式下载。

外部控制校验为按需启用：
`TEXTGUARD_CONTROL_TOOLS=<control-tools-directory> ./scripts/verify_textguard.sh --group control --evidence <new-external-directory>`。
它校验两个持久化助手与仓库实现一致，然后只在临时沙箱中运行 checkpoint 恢复测试。外部助手
缺失或不一致时给出 BLOCKED/2。普通运行器结果把外部控制校验标为 NOT_APPLICABLE，
且从不要求该目录存在。

Checkpoint API：`coordinator_lock(control_root, session_id)` 校验进程身份；
`save(event, update, control_root)` 以 PREPARED/COMMITTED 事件与保留的原始备份，序列化一次
原子状态替换。`migrate_v2` 返回保留身份、遗留字段与重试预算的副本。它不写入。
`reconcile(state, current_fingerprint)` 是显式的保守安全演练：它把单一指纹应用到每个 PASS，
因此不相关的基线阶段可能变为 NOT_RUN/STALE。不要在协调器启动时自动调用它，也不要在没有
surface/phase 影响映射的情况下把其输出当作实际恢复。Schema 3 保持为 3。
合成测试传入临时控制根；不修改任何真实状态。既有的 schema-3 相对引用按所提供的控制根解析。
保存不相关事件会保留状态与重试预算，而每个 PASS 仍要求存在证据，包括历史摘要。缺失的证明
必须由协调器回填；save 永不为历史 PASS 提供豁免。迁移助手同样执行这一保守演练，并要求在其
返回的副本用于在线协调器之前完成显式的恢复审查。
