# 第七轮评审：imboy 产品侧 TextGuard 漂移审计（2026-10-20，session4）

范围：f3ca4fbf（attempt4 EUnit 证据绑定头）→ a2fe60cd（imboy 集成候选头）。

## 事实
- 两提交间共 16 个 commit；TextGuard 相关路径唯一变化 = `src/textguard_pb.erl` 被新提交跟踪（+10872 行，纯新增；`src/textguard.proto` 零变化）。
- 35b41e78（textguard 迁移重编号 00000197）是候选祖先 ✓（先于 f3ca4fbf 合并，证据已覆盖）。

## 生成件功能等价性证明
- 提交版 pb（1b813e99）≠ Makefile 规范再生（397c20bc，与 EUnit 运行时再生 local-regen 字节一致）：差异为导出分组布局与版本注释（不同 gpb 调用路径，同为 gpb 5.0.0）。
- 全量导出函数集 diff = 空；消息表示均为 maps；
- **决定性**：CheckTextRequest（含 enum + repeated 嵌套 + UTF-8 文本）编码输出**字节级一致**。
- 结论：功能零漂移，EUnit 证据实质覆盖候选头；manifest 的 evidence_binding_head=f3ca4fbf 表述保持诚实，本轮补记等价性证明。

## 卫生备注（用户域，非阻断）
- 提交版 pb 来自非 Makefile 调用路径（缺 use_packages 的标准产物布局）；建议用户后续用 make 再生以获得字节规范跟踪，不构成交付缺陷。

## 冻结验证入口
- verify_textguard.sh --list 在无证据环境时 BLOCKED + 退出码 2，精确符合 §15.3；交付期间由冻结 commands.json 驱动 ✓

## 自纠记录
- 审计中我的探针曾用错误消息名（CheckRequest，实为 CheckTextRequest）导致四次误报 case_clause，已纠正——探针错误非产品缺陷。

资源清理完成，无状态变更。
