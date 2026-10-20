# 第五轮评审功能闭环冒烟收据（2026-10-20，session4）

候选 49dfe3f 真实二进制（macOS，fixture 表 test-t2s-v1，合成词库，临时 secrets 已销毁）。

## 发布闭环
- 空 store etag 获取 → PUT（If-Match CAS）→ generation 1 / ready:true / scene_counts 3×3 ✓；ready 503→200 翻转 ✓
- 快照身份合同：规则集 t2s_table_id 与服务启动表不匹配 → UNSUPPORTED_RULESET 拒绝 ✓（409 CAS/If-Match 缺失同样拒绝）
- 词库表错配用真实表重启后发布成功——身份校验按设计工作

## 检测探针（全部合同正确）
1. 别名 ceshici（规则 severity=high，别名 action=review）→ decision=review，effective_action=review —— 决策/动作与严重度分离 ✓
2. canonical 测试词 high → block，start_byte=6 原文字节偏移 ✓
3. canonical 演示词 medium → review ✓
4. 例外视图域：短语 演示词报告、views=["B"] → 命中经 T 视图见证存活（review，match_methods=[literal,t2s]）；扩展 views=["B","T"] → allow hits=0 —— "例外不能全段放行、按视图精确豁免"合同语义运行时证明 ✓
5. 无命中 → allow，scan_complete=true ✓
6. 双语义：profile_field(compat-v1) 别名不编译（allow）而 canonical 仍 block；channel_message(enhanced-v1) 两者皆生效 ✓

## 三端漂移审计
- imboyapp 自 2215ef2d、imboyadmin 自 a2160f46 以来零审核合同路径变更 —— UNCHANGED_VERIFIED 在当前 HEAD 成立 ✓

## 处置
- 零缺陷、零状态变更。评审 worktree/secrets 已清理。
