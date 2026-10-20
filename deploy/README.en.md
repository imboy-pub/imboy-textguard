# Local Linux container candidate

[中文](README.md) | [English](README.en.md)

This directory is the deployment skeleton; no production deployment is authorized for this
round. The service only accepts loopback plaintext listeners. The Linux container uses host
networking; cross-host detection must go through a TLS gateway on the same Linux host, and
clients verify the trusted CA, hostname and certificate validity. `nginx-tls.conf` is a
standalone gateway example that exposes only the two detection entrypoints; management,
health and metrics stay on loopback and are never proxied to the public internet.
Management HTTP 8083 uses the separate PUBLISH credential; detection HTTP 8081 / gRPC 8082
use CHECK.

First run `cargo build --locked --release` on a Linux local filesystem with the pinned
toolchain. Put the artifact for the same architecture at `build/imboy-textguard` in a
temporary build context, copy this directory's Dockerfile, and run
`docker build -f Dockerfile -t textguard:local .` in that context.
The runtime base is Debian trixie at a fixed digest; never put a macOS binary into the image.
Record the build SHA, Cargo.lock digest, CPU architecture and image ID before running
container verification.

The deployment operator prepares the private volumes in advance: the two regular files
`check` and `publish` must be readable by UID 10001 with mode 0600, each an independent
random 32–4096-byte visible ASCII secret; example values must not be used.
The volume is mounted read-only at `/run/secrets`; secrets never enter the image, command
arguments, environment values or logs. The rules volume is owned by UID 10001, directory
mode 0700, and must be a Linux local persistent filesystem.
On first start, with no rules published, live=200 and ready=503; publish a complete package
with PUBLISH before accepting detection traffic.

`TEXTGUARD_IMAGE` points at the verified immutable image ID/digest, `TEXTGUARD_SECRET_VOLUME`
at the volume described above. `docker compose -f deploy/compose.yaml up -d` applies to an
authorized isolated Linux environment.
Default quota is 2 vCPU / 2 GiB, non-root, read-only root, no capabilities, 64 processes.
HTTP connections 128, gRPC global streams 128, shared readers 64, workers 2, waiting 32,
wait 5ms, check 80ms.
Small-body receive 1s, management upload 30s, publish 60s; responses and actual socket
writes use a 1s deadline.
SIGTERM stops accepting and drains connections, exiting with an error if draining is not
complete within 65s; the container grace period is 70s.

`imboy-textguard --healthcheck` probes local live; readiness uses `GET /health/ready`.
For standard gRPC Health, an empty service name and `imboy.textguard.v1.TextGuard` mean
readiness, while `liveness` means the process is alive; readiness state is refreshed at most
every 200ms.
`GET http://127.0.0.1:8083/metrics` is for local monitoring only, with fixed low-cardinality
labels; it must never be reverse-proxied to the public internet.
Error logs print only fixed error codes; access logging of request/response bodies,
authorization headers and URI parameters is disabled.
To roll back, remove detection traffic first, stop this isolated instance, and keep the
rules volume and the old image; run the rules rollback acceptance before restoring the old
candidate.
Full TLS, bad-certificate and actual fault acceptance belong to T23; full capacity and the
one-hour soak belong to T24.
