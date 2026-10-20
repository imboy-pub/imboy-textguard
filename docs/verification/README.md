# TextGuard V2.1 交付验证档案

[中文](README.md) | [English](README.en.md)

本目录是 TextGuard MVP（计划 V2.1，run-20261016T075111Z）交付验证的**归档证明**：
产物合格 + 关键工程决策的依据。原始逐请求测试数据（约 17G）留存于仓外证据区并于
2026-10-20 按用户决定清除；本目录保存全部判定文件与结论链。

## 交付状态

`PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS` —— TextGuard 全范围验收绿；与满分的差距
全部来自授权外/用户域的既有事项（见"声明残余"）。

| 门禁 | 结果 | 说明 |
|---|---|---|
| G0—G4 基线/引擎/双协议/发布/业务 | PASS | Erlang↔Rust 真实 HTTP/gRPC 闭环 |
| G5 资源容量 | PASS_WITH_DECLARED_SOAK_RESIDUAL | 500QPS p99 3.9ms；过载 1500QPS 零意外；soak 内存残余已文档化（分配器高水位，非泄漏，A/B 证据 932→222MB） |
| G6 内容 | PASS_WITH_DECLARED_LIMITS | restricted_local：无业务授权语料，新增 block 规则强制 review/shadow |
| G7 恢复 | PASS | 干净安装 + 三类回滚 |
| G8 发布 | PASS_WITH_DECLARED_RESIDUALS | 全量 EUnit 的非 TextGuard 既有失败（native 门 22 项为 GitHub-runner-only 设计 + 4 个用户域行为单例） |
| G9 生产 | NOT_AUTHORIZED | 未获授权 |
| 上游 U 线 | NO_UPSTREAM_CHANGE_NEEDED | 无通用 aho-corasick 缺陷，产品消费官方发布版未打补丁 |

## 关键实测数字（真实 Linux 容器，2 vCPU / 2GiB）

- B1 增强基线（5 万 canonical + 1 万别名 + 5 千 gap + 32 regex + 100 例外）：500 完成请求/秒，
  p99 3.9—4.7ms；过载阶梯 750/1000/1500 QPS unexpected=0（1500 唯一失败为 HTTP_429 有界拒绝）
- 1 小时 soak：180 万请求 p99 3.4ms，100/100 规则发布
- 全量 EUnit（imboy f3ca4fbf）：24/24 分块两轮 pass，TextGuard 9/9 套件绿（chunk-23 Passed:841）
- 运行时冒烟（R4—R9）：启动 fail-closed、401/400/503 语义、60 并发零错误、CAS 三向、
  gRPC 线缆 15/15、持久恢复与灾难 fail-closed、八项合同行为

## 候选与血缘

- 服务：`imboy-textguard` integration **49dfe3f**（aho-corasick =1.1.3 registry 锁定；该分支
  已合并入 main=429293e，main 另含用户要求的 =1.1.5 升级，容量证据未绑定 1.1.5）
- 消费端：`imboy` integration **a2fe60cd**（= 合并时 main tip；迁移 00000197_textguard_review_metadata）
- 边界：未 push、未投稿（U05）、未部署生产（T29）

## 声明残余与解锁条件

1. AC-095 soak 内存残余：分配器高水位 ~3.4MiB/pub（文档化，非泄漏）；
2. G6：业务标注语料授权后才能启用新增 block；
3. G8：GitHub runner 跑绿 native 门 + 处理 4 个用户域既有单例后重跑全量 EUnit。

## 文件索引

- `release-manifest.json` — 候选指纹/依赖锁定/门禁终态（权威）
- `runbook.md` — 构建/启动/发布/检测/回滚/恢复操作手册（全部实测命令）
- `decisions-ledger.md` — 执行决策流水（含证据区路径的历史引用）
- `reviews/` — 独立工程审查与九轮评审收据（R4—R9 为运行时/线缆/漂移/卫生审计）
- `evidence/` — 关键判定文件：容量观测、soak、A/B、语料评估、EUnit 汇总、上游边界、追溯快照
