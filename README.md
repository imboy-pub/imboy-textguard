# IMBoy TextGuard

独立 Rust 文本检测服务。本地实施候选，尚未完成产品验收。

运行时不依赖相邻 aho-corasick checkout；依赖使用公开 registry 版本及本仓 Cargo.lock。
需要 Rust 1.90.0（含 rustfmt、clippy）和 protoc。

```sh
cargo build --locked --release
cargo test --locked --test contract
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- --version
```

启动前用独立的私密 CHECK/PUBLISH 文件和本地规则目录配置服务：

```sh
TEXTGUARD_DATA_DIR=/absolute/private/rules \
TEXTGUARD_CHECK_SECRET_FILE=/absolute/private/check \
TEXTGUARD_PUBLISH_SECRET_FILE=/absolute/private/publish \
cargo run --locked --release
```

文件权限0600，每个secret为独立的32–4096字节可见ASCII随机值。
默认loopback HTTP8081/gRPC8082/管理8083；管理口GET/PUT `/internal/v1/rulesets/active`
执行完整规则包CAS发布。未发布时ready=503，检测返回NOT_READY；检查失败不会返回allow。
两种检测协议共享worker/队列和80ms预算。容器、TLS gateway、指标与退出说明见 `deploy/README.md`。
T17组件通过不代表真实Erlang闭环、Linux容量和完整故障验收完成。
公开投稿与生产部署未获授权。
