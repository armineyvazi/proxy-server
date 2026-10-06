# proxy-server

An HTTP/1.1 reverse proxy built for the ArvanCloud systems challenge.
Written in Rust using only raw POSIX primitives — no async runtime, no HTTP library.

---

## Acknowledgments

Before anything else, I would like to sincerely thank you for designing and sharing this challenge.

I genuinely enjoyed working on it. Throughout the process, I learned a lot, discovered several areas where my knowledge was lacking, and gained a much deeper understanding of systems design, proxy architectures, performance analysis, and debugging methodologies.

I also appreciate the time you spend reviewing submissions and reading through candidates' projects. That effort is often invisible to participants, but it is something I truly value.

In fact, I enjoyed this project so much that I plan to continue developing it even after completing the assignment.

---

## Design Process

The design was informed by studying how existing high-performance proxies — NGINX and Pingora in particular — make their architectural trade-offs, then applying the lessons that fit this project's constraints rather than copying an implementation outright. The full research notes are in the Notion workspace linked at the [end of this README](#further-reading).

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Client (curl / browser)                          │
└─────────────────────────────┬───────────────────────────────────────────┘
                              │  TCP :80
                              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          Proxy host / VM                                │
│                                                                         │
│  ┌──────────────────────────────────────────────────┐                  │
│  │              supervisor (main.rs)                │                  │
│  │  • counts CPUs via sysconf(_SC_NPROCESSORS_ONLN) │                  │
│  │  • mmap(MAP_SHARED|MAP_ANON) → shared LogBuffer  │                  │
│  │  • fork() × N workers                            │                  │
│  │  • waitpid(-1, WNOHANG) every 100 ms             │                  │
│  │    → crash detected → fork() replacement         │                  │
│  └──────────┬───────────────────────────────────────┘                  │
│             │ fork()                                                    │
│    ┌────────┴────────────────────────────────────────┐                 │
│    │  worker[0]   worker[1]   worker[2]  worker[N-1] │                 │
│    │  CPU 0       CPU 1       CPU 2      CPU N-1      │                 │
│    │                                                  │                 │
│    │  each worker:                                    │                 │
│    │    SO_REUSEPORT bind :8080                       │                 │
│    │    mio::Poll (epoll)                             │                 │
│    │    ┌─────────────────────────────────────────┐  │                 │
│    │    │ connection state machine                 │  │                 │
│    │    │  ReadingRequest → ConnectingUpstream     │  │                 │
│    │    │       → Forwarding → (pool or close)     │  │                 │
│    │    │  /.svc/collect_logs → CollectLogs phase  │  │                 │
│    │    └─────────────────────────────────────────┘  │                 │
│    │    keepalive pool: up to 64 upstream sockets     │                 │
│    │    shared LogBuffer (mmap, pthread mutex)        │                 │
│    └────────────────────────────────────────────────-┘                 │
│                                                                         │
│  ┌──────────────────────────────────┐                                  │
│  │  loadtest (Go, port 8090)        │  ← web UI + Prometheus /metrics  │
│  └──────────────────────────────────┘                                  │
│  ┌──────────────────────────────────┐                                  │
│  │  upstream  (Go, port 9000)       │  ← benchmark target (Docker)     │
│  └──────────────────────────────────┘                                  │
│  ┌───────────────────────────────────────────────────┐                 │
│  │  node-exporter :9100  Prometheus :9090  Grafana :3000│              │
│  └───────────────────────────────────────────────────┘                 │
└─────────────────────────────────────────────────────────────────────────┘
```

The system is designed around a simple but extensible architecture.

At a high level:

* The proxy server is responsible for accepting incoming connections and forwarding traffic to upstream servers.
* A monitoring service continuously collects operational metrics and runtime statistics.
* A dedicated load-testing service generates traffic and allows performance validation under different workloads.
* All components are isolated to minimize interference during testing and benchmarking.

The architecture was intentionally kept modular so that individual components can evolve independently as new features and experiments are added.

---

## Approach

Observability and testing infrastructure were built *before* the proxy itself — a lightweight monitoring stack (Prometheus + Grafana + node-exporter) and a custom load-testing service with a web UI, ramp mode, and mixed-methods scenarios, run from isolated VMs to keep measurements free of resource contention between components. Rust was chosen over C for memory safety: it allows focusing on architecture, networking, and performance without spending the project budget on manual memory-management correctness. Testing then drove most of the engineering — exploring traffic patterns and failure scenarios surfaced several real bugs, each documented with root cause, reproduction, impact, and fix.

The full reasoning behind these choices, along with research notes and the complete bug write-ups, lives in the Notion workspace — see [Further Reading](#further-reading).

---

## Build

```bash
cargo build --release
```

Requires: Rust 1.85+ (edition 2024; built and tested on 1.91), Linux (Debian 10+)
for full functionality. Compiles on macOS for development; `sched_setaffinity` is
a no-op there. The load-test and upstream services are Go 1.26.

---

## Run

```bash
./target/release/proxy-server --inbound 0.0.0.0:8080 --outbound example.com
```

| Flag | Description |
|---|---|
| `--inbound <addr:port>` | Address and port to listen on |
| `--outbound <host[:port]>` | Upstream host to forward to (default port 80) |

The proxy forks one worker per logical CPU core and listens immediately.

---

## Running the Project (Full Stack)

The complete stack — proxy, upstream, loadtest UI, Prometheus, Grafana — is managed via Ansible + Docker Compose.

### Prerequisites

- Ansible installed locally
- SSH access to the target VM
- Inventory configured from `ansible/inventory.ini.example` → `ansible/inventory.ini`

### Deploy

```bash
# Deploy the full stack (proxy-server mode)
ansible-playbook -i ansible/inventory.ini ansible/playbook.yml

# Deploy only the proxy role (re-build and restart proxy-server)
ansible-playbook -i ansible/inventory.ini ansible/playbook.yml --tags proxy

# Switch from proxy-server to nginx for comparison
ansible-playbook -i ansible/inventory.ini ansible/switch.yml
```

### Service endpoints (after deploy)

| Service | URL | Notes |
|---|---|---|
| Proxy | `http://<vm-ip>/` | Port 80 → container 8080 |
| collect_logs | `http://<vm-ip>/.svc/collect_logs` | |
| Load test UI | `http://<vm-ip>:8090/` | Web dashboard |
| Prometheus | `http://<vm-ip>:9090/` | |
| Grafana | `http://<vm-ip>:3000/` | admin / ProxyBench2024! |
| node-exporter | `http://<vm-ip>:9100/metrics` | |

### Run locally (Docker Compose only)

```bash
# Start proxy-server + upstream + loadtest + monitoring
docker compose --profile proxy-server up -d --build

# Switch to nginx for comparison
docker compose stop proxy-server
docker compose --profile nginx up -d

# Run a quick load test from the CLI
cd loadtest && go run . \
  --url http://localhost/ \
  --host upstream \
  --concurrency 200 \
  --duration 60
```

### Run tests

```bash
cargo test
```

Integration tests spin up the proxy binary against mock upstreams. Each test uses a unique fixed port to support parallel execution.

---

## Architecture

**Multi-process prefork with SO_REUSEPORT.** The supervisor process (`main.rs`) reads the CPU count via `sysconf(_SC_NPROCESSORS_ONLN)`, allocates a shared mmap log buffer, then `fork()`s one worker per core. Each worker independently binds the inbound address with `SO_REUSEPORT` set before `bind()`. The Linux kernel distributes incoming connections across all workers using a hash of `(src_ip, src_port)` — zero coordination overhead, no single accept bottleneck.

**Per-worker event loop.** Each worker runs a single-threaded `mio::Poll` (epoll) event loop. Non-blocking `accept()` + non-blocking TCP connect to upstream + bidirectional streaming — one worker handles thousands of concurrent connections without threads. Connection state (phase, buffers, socket pair) is tracked in a `HashMap<usize, Conn>` keyed by a connection ID encoded into the epoll token. The token encoding `(conn_id * 2)` for client and `(conn_id * 2 + 1)` for upstream gives O(1) lookup of both the connection and the active socket.

**Crash resilience.** The supervisor runs a `waitpid(-1, WNOHANG)` poll loop every 100 ms. If any worker exits (crash, OOM, signal), the supervisor detects it, logs the exit reason, and immediately `fork()`s a replacement pinned to the same CPU. The other workers continue serving traffic uninterrupted.

---

## collect_logs endpoint

```
GET /.svc/collect_logs HTTP/1.1
Host: <any>
```

Returns all proxied request logs since the last call, one per line:

```
1718000001 192.168.1.10 73180
1718000002 10.0.0.5 73181
1718000003 172.16.0.1 73182
```

Format: `<unix_timestamp> <client_ip> <worker_pid>`

A second call with no new traffic returns an empty body (HTTP 200, Content-Length: 0).
The drain is atomic — guaranteed no duplicate entries even with concurrent workers.

```bash
# Example
curl http://localhost:8080/.svc/collect_logs
```

The log buffer is a ring of 65 536 entries in shared memory. At 10 000 req/s, this
provides over 6 seconds of history between drains.

---

## Design decisions

**SO_REUSEPORT instead of accept-dispatch.** The alternative — one process accepts connections and hands fds to workers via `sendmsg(SCM_RIGHTS)` — creates a single-threaded bottleneck at the accept step and adds latency for every connection. SO_REUSEPORT offloads load balancing to the kernel, which does it with a per-socket accept queue and no cross-process coordination.

**mmap MAP_SHARED for log storage.** After `fork()`, each process has its own copy of the virtual address space. A normal `Vec` or `Mutex<T>` in the parent is invisible to children. `mmap(MAP_SHARED | MAP_ANON)` creates one physical page mapped into every process's address space — writes in any child are immediately visible to all others. This is the only mechanism that works across `fork()` without an IPC round-trip.

**PTHREAD_PROCESS_SHARED mutex.** A default `pthread_mutex_t` uses userspace-only locking that is not visible across process boundaries. Setting `PTHREAD_PROCESS_SHARED` switches to a futex-based implementation keyed on the physical memory address — works correctly regardless of which process holds the lock.

**httparse for zero-copy parsing.** httparse returns `&[u8]` slices into the original read buffer — no allocation, no copying per header. At high request rates, allocating one `String` per header would create millions of short-lived allocations per second. Zero-copy parsing keeps the hot path allocation-free.

**Upstream DNS resolved once at startup.** `getaddrinfo()` (the DNS resolver) is a blocking call that can take 10–300 ms. Calling it inside the epoll event loop on every connection would stall all connections on that worker for the duration of the DNS round trip. The upstream `SocketAddr` is resolved once in `run_worker()` at startup and passed through the event loop by value.

**Per-worker upstream keepalive pool (64 slots).** Mirroring nginx's `keepalive 64`, each worker maintains a pool of idle upstream TCP connections. When a response with `Content-Length` completes, the socket is deregistered from epoll and parked in the pool. The next request pops it, skipping the TCP 3-way handshake (~0.3–1 ms on LAN). Chunked responses and upstream-close responses are never pooled because the socket position is unknown without a full decoder. If a pooled socket turns out to have been closed by the peer — whether that surfaces as immediate EOF on the first read or as a write error / RST — the request is transparently resent once on a fresh connection rather than returning an empty or failed reply.

**Response-path backpressure.** A slow client cannot make a worker buffer an unbounded response. Reads from the upstream pause once `resp_buf` passes a high-water mark (256 KB) and resume when the client drains below the low-water mark (64 KB). Because epoll is edge-triggered, the resume is driven directly from the client-writable path rather than waiting for an upstream event that would never arrive. Partial writes to the client are retried on `WRITABLE`, so responses are never truncated.

**Client keepalive and request framing.** Client connections are reused across requests when the client asks for it and the response is self-delimiting (`Content-Length`, chunked, or a HEAD). Request bodies are framed precisely: `Content-Length` and chunked are honored, chunked framing is preserved to the upstream, and smuggling vectors — conflicting `Content-Length`, `Content-Length` with `Transfer-Encoding`, or a non-`chunked` final coding — are rejected with 400.

**Abuse bounds.** Requests that exceed the header-size limit get 431; connections that stall mid-request are closed (408) after a header timeout, and idle connections after an idle timeout. These guard against header floods and slowloris. Worker count and timeouts are overridable via `PROXY_WORKERS`, `PROXY_HEADER_MS`, and `PROXY_IDLE_MS`.

---

## Project structure

```
proxy-server/
├── Cargo.toml           # edition 2024
├── clippy.toml          # Rust linter config
├── rustfmt.toml         # Rust formatter config
├── Dockerfile
├── docker-compose.yml
├── src/
│   ├── main.rs          # Supervisor: parse args, fork workers, respawn on crash
│   ├── worker.rs        # Per-process epoll event loop + keepalive pool
│   ├── proxy.rs         # HTTP parse, Host validation, request rewrite
│   ├── logs.rs          # Shared ring-buffer read/write
│   └── ipc.rs           # mmap region + process-shared mutex
├── tests/
│   └── integration.rs   # End-to-end tests against mock upstreams
├── loadtest/            # Go 1.26 load generator
│   ├── main.go          # Entry point + HTTP routes
│   ├── config.go        # RunConfig + presets
│   ├── stats.go         # Sharded latency/code accumulation, histogram, ring
│   ├── runner.go        # Worker pool, ramp mode, rate limiter, request loop
│   ├── api.go           # JSON API handlers + app state
│   ├── metrics.go       # Prometheus /metrics
│   ├── ui.go            # Embedded web dashboard
│   └── .golangci.yml    # Go linter config (v2)
├── docker/
│   ├── upstream/        # Benchmark target (Go HTTP server)
│   ├── prometheus/      # prometheus.yml + rules.yml (recording rules + alerts)
│   ├── grafana/         # provisioned datasource + dashboards
│   └── nginx/           # nginx config for the comparison baseline
└── ansible/             # deploy + backend-switch playbooks
```

---

## Observability

The benchmark is measured **client-side** by the load tester, which exposes Prometheus metrics on `:8090/metrics`:

- `loadtest_requests_total{code,method,scenario}` — request counter, the basis for RPS (`rate(...)`) and the error ratio.
- `loadtest_latency_seconds` — a histogram with buckets out to 10 s. This is the source of truth for latency: `histogram_quantile(0.99, sum(rate(loadtest_latency_seconds_bucket[1m])) by (le))` gives a windowed p99 that reflects *current* behaviour.
- `loadtest_errors_by_type{type}` — splits failures into `network` (transport), `gateway` (502/503/504), `upstream` (other 5xx), and `client` (4xx), which answers "is it the proxy or the upstream?".
- `loadtest_rps` / `loadtest_latency_p*_seconds` — cumulative-since-test-start gauges. Convenient as single current-value readouts, but they flatten over a run, so trend panels and alerts use the histogram and the recording rules instead.

`docker/prometheus/rules.yml` adds windowed recording rules (`loadtest:requests:rate1m`, `loadtest:errors:ratio1m`, `loadtest:latency_seconds:p99_1m`) and alerts for high error rate, p99 SLA breach, target down, host CPU saturation, and file-descriptor exhaustion. Host-level CPU, memory, network, TCP, and FD metrics come from node-exporter (mounted on the host `/proc` and `/sys`).

Two deliberate caveats: latency quantiles are computed over **successful** requests only (timeouts are counted as errors, not slow samples), and the proxy itself exposes no `/metrics` — server-side RED metrics would be the natural next step (a `/.svc/metrics` endpoint analogous to `collect_logs`).

---

## Known limitations

- **No TLS.** Both inbound and upstream connections are plain TCP.
- **IPv4 only.** IPv6 addresses are not supported.
- **Chunked upstream responses are not pooled.** They are forwarded correctly and detected for completion via a terminal-chunk scan restricted to the response body (so a header value ending in `0` cannot false-trigger completion), but the socket is not returned to the keepalive pool (that would need a full chunked decoder to know the exact byte position after the terminator).
- **No HTTP request pipelining.** A client connection is reused for sequential keep-alive requests, but bytes for a second request sent before the first response are not handled; such a connection is closed instead of reused.
- **No server-side proxy metrics.** Request-level RED metrics are measured client-side by the load tester; the proxy itself exposes no `/metrics`.
- **Fixed-size log ring buffer.** If more than 65 536 requests arrive between `collect_logs` calls, the oldest entries are silently overwritten.
- **macOS only for development.** `sched_setaffinity` CPU pinning is Linux-only and is a no-op on macOS. Run in production on Linux/Debian 10+.
- **No HTTP/2 or HTTP/3.** This is an HTTP/1.1 proxy only.

---

## Further Reading

Design decisions, architecture notes, bug write-ups, benchmarks, and the engineering narrative live in the project's Obsidian documentation (Persian technical reference). The published source in this repository is the source of truth for implementation details.
