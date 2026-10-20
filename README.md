# IMBoy TextGuard

[中文](README.md) | [English](README.en.md)

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

## 许可证

本项目采用[木兰宽松许可证，第2版](http://license.coscl.org.cn/MulanPSL2)（Mulan PSL v2）授权，
SPDX 标识符 `MulanPSL-2.0`，全文见仓库根目录 `LICENSE`。该许可证为中英文双语，
两版具有同等法律效力，如有冲突以中文版为准。

第三方依赖各自保留原有许可证（均为 MIT、Apache-2.0、BSD 等宽松许可，无 copyleft 传染）；
`data/` 与 `tests/oracle/` 下的 Unicode 数据文件遵循其自带的 Unicode 许可声明。

IMBoy 生态各仓许可证相互独立：Flutter 客户端采用木兰宽松许可证第 2 版，
后端与管理后台采用 BSL 1.1（含使用范围限制）。具体使用须阅读对应仓库的 `LICENSE`。
