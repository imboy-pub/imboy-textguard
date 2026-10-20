# 第六轮评审：持久恢复 + gRPC 线缆冒烟收据（2026-10-20，session4）

[中文](R6-recovery-grpc-smoke.md) | [English](R6-recovery-grpc-smoke.en.md)

候选 49dfe3f 真实二进制（macOS，fixture 表，合成词库，临时 secrets 已销毁）。

## gRPC 线缆级
tests/grpc_contract.rs（真实 TcpListener + tonic Channel 过网）：15 passed / 0 failed ✓

## §9.4 持久恢复
- 发布 generation 1（rev 5fea107f...，instance 782520a7...）→ 杀进程 → 重启同数据目录：
  ready=200（免重发布）、revision 精确一致、instance-id SAME、检测仍工作（测试词→block/1001）✓
## 灾难 fail-closed
- 删除 ACTIVE manifest（manifest.json）→ 重启：ready=503，检测返回 NOT_READY retryable —— 不崩溃、绝不返回 allow ✓

资源清理完成，无状态变更。
