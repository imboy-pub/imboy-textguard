# 本地 Linux 容器候选

本目录是部署骨架；本次没有生产部署授权。服务只接受 loopback 明文监听。
Linux 容器使用 host network；跨主机检测必须经同一 Linux 主机上的 TLS gateway，
客户端验证受信 CA、主机名和证书有效期。`nginx-tls.conf` 是独立 gateway 样例，
只公开两个检测入口；管理、health 和 metrics 留在 loopback，不代理到公网。
管理 HTTP 8083 使用独立 PUBLISH 凭证；检测 HTTP 8081 / gRPC 8082 使用 CHECK。

先用锁定工具链在 Linux 本地文件系统完成 `cargo build --locked --release`。
将同架构产物放在临时构建上下文的 `build/imboy-textguard`，复制本目录 Dockerfile，
在该上下文运行 `docker build -f Dockerfile -t textguard:local .`。
运行依赖是固定 digest 的 Debian trixie；不要将 macOS 二进制放进镜像。
保存构建 SHA、Cargo.lock 摘要、CPU 架构和镜像 ID 后才运行容器验证。

私密卷由部署操作者预先准备：`check`、`publish` 两个普通文件必须 UID 10001 可读、
权限 0600，各为独立的 32–4096 字节可见 ASCII 随机 secret；不能使用示例值。
卷只读挂在 `/run/secrets`，secret 不进入镜像、命令参数、环境值或日志。
规则卷 UID 10001、目录 0700，必须是 Linux 本地持久文件系统。
首次启动未发布规则时 live=200、ready=503；先用 PUBLISH 发布完整包再接检测流量。

`TEXTGUARD_IMAGE` 指向已验不可变镜像 ID/digest，`TEXTGUARD_SECRET_VOLUME` 指向上述卷。
`docker compose -f deploy/compose.yaml up -d` 适用于已授权的隔离 Linux 环境。
默认配额 2 vCPU/2 GiB、非 root、只读根、无 capabilities、进程数64。
HTTP连接128、gRPC全局streams128、共享reader64、worker2、等待32，等待5ms、检查80ms。
小body接收1s、管理上传30s、发布60s；响应和实际socket写入设置1s截止。
SIGTERM 停止接收并排空连接，65s未完成报错退出；容器宽限70s。

`imboy-textguard --healthcheck` 探测本机 live；ready 使用 `GET /health/ready`。
标准 gRPC Health 的空服务名与 `imboy.textguard.v1.TextGuard` 表示 readiness，
`liveness` 表示进程存活；就绪状态最多200ms更新。
`GET http://127.0.0.1:8083/metrics` 仅供本机监控，固定低基数标签，禁止反向代理公网。
错误日志只打印固定错误码；不启用请求/响应正文、authorization、URI参数的访问日志。
回退时先撤检测流量、停止本隔离实例，保留规则卷和旧镜像；恢复旧候选前执行规则回滚验收。
完整 TLS/错误证书/实际故障验收属于 T23，完整容量与一小时 soak 属于 T24。
