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

当前骨架运行时明确返回 NOT_READY（退出 2），不提供伪造检测结果。
公开投稿与生产部署未获授权。
