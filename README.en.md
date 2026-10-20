# IMBoy TextGuard

[中文](README.md) | [English](README.en.md)

Standalone Rust text-detection service. Local implementation candidate; product acceptance is not yet complete.

The runtime does not depend on an adjacent aho-corasick checkout; dependencies come from public registry versions and this repository's `Cargo.lock`.
Requires Rust 1.90.0 (with rustfmt and clippy) and protoc.

```sh
cargo build --locked --release
cargo test --locked --test contract
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- --version
```

Before startup, configure the service with separate private CHECK/PUBLISH files and a local rules directory:

```sh
TEXTGUARD_DATA_DIR=/absolute/private/rules \
TEXTGUARD_CHECK_SECRET_FILE=/absolute/private/check \
TEXTGUARD_PUBLISH_SECRET_FILE=/absolute/private/publish \
cargo run --locked --release
```

File mode 0600; each secret is an independent random value of 32–4096 visible ASCII bytes.
Defaults are loopback HTTP 8081 / gRPC 8082 / management 8083; the management port offers
GET/PUT `/internal/v1/rulesets/active` for full ruleset CAS publication. Before the first
publication ready=503 and detection returns NOT_READY; a failed check never returns allow.
Both detection protocols share the worker, queue and the 80ms budget. Containers, the TLS
gateway, metrics and exit behaviour are documented in `deploy/README.md`.
Passing the T17 component tests does not mean the real Erlang loop, Linux capacity or full
fault acceptance is complete.
Public submission and production deployment are not authorized.

## License

This project is licensed under the
[Mulan Permissive Software License, Version 2](http://license.coscl.org.cn/MulanPSL2)
(Mulan PSL v2), SPDX identifier `MulanPSL-2.0`; the full text is in `LICENSE` at the
repository root. The license is bilingual (Chinese and English) with equal legal effect;
the Chinese version prevails in case of conflict.

Third-party dependencies keep their own licenses (all permissive: MIT, Apache-2.0, BSD and
similar, no copyleft propagation). The Unicode data files under `data/` and
`tests/oracle/` follow their bundled Unicode license notice.

Repositories in the IMBoy ecosystem are licensed independently: the Flutter client uses
Mulan PSL v2, while the backend and admin console use BSL 1.1 (with scope restrictions).
For any specific use, read the `LICENSE` of the corresponding repository.
