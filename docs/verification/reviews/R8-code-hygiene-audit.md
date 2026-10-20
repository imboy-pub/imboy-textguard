# 第八轮评审：代码卫生审计（2026-10-20，session4）

[中文](R8-code-hygiene-audit.md) | [English](R8-code-hygiene-audit.en.md)

## Rust 候选（49dfe3f，src/ 生产路径）
- dbg!：0；println!：仅 main.rs:4（--version 输出，CLI 功能）；eprintln!：仅 panic 钩子 INTERNAL_ERROR 与三处启动失败 NOT_READY（刻意失败可观测性）
- TODO/FIXME/HACK/XXX/DIAG：0
- 历史遗留：T24 复审非阻塞备注"residual DIAG eprintln"已由候选血统内提交 11dea3e（drop the leftover diagnostics block from ac088）清除

## Erlang 候选（a2fe60cd，src/moderation/）
- io:format / dbg( / TODO / FIXME / XXX：全部 0

## 其他抽查
- 候选头自第七轮零漂移（49dfe3f / a2fe60cd 不变）
- CI workflow：fmt --check + clippy -D warnings + cargo test --locked + build --locked --release（锁定、真实门禁）
- deploy/README 声明交叉核对：端口 8081/8082/8083 与凭证分离 ✓；检查预算 80ms = runtime/mod.rs:15 CHECK_BUDGET 常量 ✓；/metrics 仅 loopback ✓

零缺陷、零状态变更。
