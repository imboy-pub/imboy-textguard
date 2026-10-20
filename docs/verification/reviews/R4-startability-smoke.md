# 第四轮评审运行时冒烟收据（2026-10-20 ~06:30，session4）

候选：imboy-textguard integration@49dfe3f（aho-corasick =1.1.3 registry），macOS 开发机（仅启动性验证，非容量口径）。

- 构建：`cargo build --locked --release` 21.2s 通过；`cargo test --locked --test contract` 7 passed / 0 failed。
- 启动（临时 0600 secrets + 空词库目录，已销毁）：
  - 未认证 POST /v1/check → 401 ✓
  - 非法请求体 → 400 INVALID_ARGUMENT（deny_unknown_fields 协议校验）✓
  - 合法请求体 + 未发布 → **503 NOT_READY retryable:true（绝不返回 allow）** ✓
  - 未认证管理口 GET /internal/v1/rulesets/active → 401 ✓
  - /health/ready → 503；/health/live → 200 ✓
  - gRPC 8082 TCP 可达 ✓；store 布局（instance-id 0600/packages/tmp）正确创建 ✓
- 意外收获：数据目录位于 /tmp（符号链接祖先）时启动被拒（RULE_STORE_UNAVAILABLE）——发布库防符号链接攻击加固在真实运行中得到验证 ✓
- 清理：评审 worktree 与临时 secrets 已删除。
