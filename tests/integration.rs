// Integration test suite for proxy-server.
//
// Architecture of each test:
//   1. Bind a mock upstream on a random port (port 0 → OS assigns)
//   2. Spawn the proxy binary pointing at that mock upstream
//   3. Connect as a client and assert correct HTTP behaviour
//   4. ProxyHandle::drop() kills the proxy process + waits for cleanup
//
// Each test uses a unique fixed port so `cargo test` with multiple threads
// never produces port collisions.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::io::AsRawFd; // for SO_LINGER on the mock upstream
use std::os::unix::process::CommandExt; // for process_group(0)
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// ─── One-time global cleanup ──────────────────────────────────────────────────
// If cargo test is interrupted (Ctrl+C, timeout) ProxyHandle::drop() never runs,
// leaving worker processes bound on the fixed test ports. The next `cargo test`
// run then connects to those stale proxies (whose upstreams are gone) and gets
// 502. We kill any surviving proxy-server processes once, before the first test
// actually starts.

static SUITE_INIT: OnceLock<()> = OnceLock::new();

fn suite_setup() {
    SUITE_INIT.get_or_init(|| {
        // Exact process name only — never `pkill -f proxy-server`, which also
        // matches CI paths like `/home/runner/work/proxy-server/...` and kills
        // the test runner itself.
        let _ = Command::new("pkill")
            .args(["-9", "-x", "proxy-server"])
            .status();
        // Give the kernel time to release SO_REUSEPORT bindings.
        thread::sleep(Duration::from_millis(300));
    });
}

// ─── Port assignments (unique per test) ───────────────────────────────────────

const PORT_BASIC: &str = "127.0.0.1:18081";
const PORT_HOST_REJECT: &str = "127.0.0.1:18082";
const PORT_COLLECT_DRAIN: &str = "127.0.0.1:18083";
const PORT_CONCURRENT: &str = "127.0.0.1:18084";
const PORT_CRASH: &str = "127.0.0.1:18085";
const PORT_NO_HOST: &str = "127.0.0.1:18086";
const PORT_HOST_PORT_HDR: &str = "127.0.0.1:18087";
const PORT_POST_BODY: &str = "127.0.0.1:18088";
const PORT_EMPTY_LOGS: &str = "127.0.0.1:18089";
const PORT_LARGE_RESP: &str = "127.0.0.1:18090";
const PORT_NO_DUPS: &str = "127.0.0.1:18091";
const PORT_CONC_DRAIN: &str = "127.0.0.1:18092";
const PORT_DISTRIBUTION: &str = "127.0.0.1:18093";
const PORT_STRESS_SEQ: &str = "127.0.0.1:18094";
const PORT_STRESS_CONC: &str = "127.0.0.1:18095";
const PORT_BENCH_SEQ: &str = "127.0.0.1:18096";
const PORT_BENCH_CONC: &str = "127.0.0.1:18097";
const PORT_BENCH_LATENCY: &str = "127.0.0.1:18098";
const PORT_CHUNKED: &str = "127.0.0.1:18099";
const PORT_MANY_LOGS: &str = "127.0.0.1:18100";
const PORT_LARGE_POST: &str = "127.0.0.1:18101";
const PORT_SLOW_CLIENT: &str = "127.0.0.1:18102";
const PORT_STALE_POOL: &str = "127.0.0.1:18103";
const PORT_CLIENT_KA: &str = "127.0.0.1:18104";
const PORT_HEADER_FLOOD: &str = "127.0.0.1:18105";
const PORT_SLOWLORIS: &str = "127.0.0.1:18106";
const PORT_SMUGGLE_CL: &str = "127.0.0.1:18107";
const PORT_SMUGGLE_TE: &str = "127.0.0.1:18108";
const PORT_CHUNKED_REQ: &str = "127.0.0.1:18109";
const PORT_CHUNKED_HDR_ZERO: &str = "127.0.0.1:18110";
const PORT_STALE_POOL_RST: &str = "127.0.0.1:18111";

const LOG_REQ: &str =
    "GET /.svc/collect_logs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

// ─── RAII proxy guard ─────────────────────────────────────────────────────────

/// Kills the proxy supervisor AND all its forked worker processes on drop.
///
/// Simply killing the supervisor leaves workers running as orphans (reparented
/// to init). Those stale workers stay bound on the SO_REUSEPORT port and steal
/// connections from the next test's proxy, returning 502 because their upstream
/// is gone. Fix: start the proxy in its own process group so that
/// `kill(-pgid, SIGKILL)` terminates the entire group atomically.
struct ProxyHandle(Child);

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        let pgid = self.0.id() as libc::pid_t;
        unsafe {
            // SAFETY: pgid is valid (child we own) and equals the process group
            // ID because spawn_proxy used process_group(0). Negative first arg
            // means "send to every process in the group".
            libc::kill(-pgid, libc::SIGKILL);
        }
        let _ = self.0.kill(); // belt-and-suspenders for the supervisor itself
        let _ = self.0.wait();
        thread::sleep(Duration::from_millis(50)); // let the kernel reap workers
    }
}

// ─── Test helpers ─────────────────────────────────────────────────────────────

fn random_listener() -> (TcpListener, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind random port");
    let addr = l.local_addr().expect("local_addr");
    (l, addr)
}

fn spawn_proxy(inbound: &str, outbound: &str) -> ProxyHandle {
    spawn_proxy_env(inbound, outbound, &[])
}

/// Spawn the proxy with extra environment variables. Tests use `PROXY_WORKERS=1`
/// to make connection pooling and keep-alive deterministic, and `PROXY_HEADER_MS`
/// to exercise the slowloris timeout quickly.
fn spawn_proxy_env(inbound: &str, outbound: &str, env: &[(&str, &str)]) -> ProxyHandle {
    suite_setup(); // kill stale processes from any previous interrupted run

    let bin = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent1")
        .parent()
        .expect("parent2")
        .join("proxy-server");

    let mut cmd = Command::new(&bin);
    cmd.arg("--inbound")
        .arg(inbound)
        .arg("--outbound")
        .arg(outbound)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0); // give proxy its own PGID so drop() kills ALL workers
    for (k, v) in env {
        cmd.env(k, v);
    }
    let child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {:?} failed: {}", bin, e));

    // Poll until the proxy is actually accepting connections rather than sleeping
    // a fixed interval. With 18 tests running in parallel each forking N workers,
    // a fixed 300 ms is too short on loaded machines but too long on fast ones.
    let addr: std::net::SocketAddr = inbound
        .parse()
        .unwrap_or_else(|_| panic!("invalid inbound address: {}", inbound));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(50)) {
            // Probe connection closes immediately — proxy reads 0 bytes and drops it
            // without logging (log_request is only called after a full proxy cycle).
            Ok(_) => break,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("proxy on {} did not start within 10s: {}", inbound, e),
        }
    }
    ProxyHandle(child)
}

fn http_get(addr: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).expect("write request");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok();
    resp
}

fn http_get_timed(addr: &str, request: &str) -> (String, Duration) {
    let t = Instant::now();
    let resp = http_get(addr, request);
    (resp, t.elapsed())
}

fn status_code(resp: &str) -> u16 {
    resp.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn response_body(resp: &str) -> &str {
    resp.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn count_log_lines(resp: &str) -> usize {
    response_body(resp)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

fn parse_pids(resp: &str) -> Vec<u32> {
    response_body(resp)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().nth(2)?.parse().ok())
        .collect()
}

fn percentile(mut v: Vec<u128>, p: usize) -> u128 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() * p) / 100).min(v.len() - 1)]
}

fn num_logical_cpus() -> usize {
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n <= 0 { 1 } else { n as usize }
}

// ─── Mock upstream factories ──────────────────────────────────────────────────

fn start_mock_upstream(response: &'static str) -> SocketAddr {
    let (listener, addr) = random_listener();
    thread::spawn(move || {
        for stream in listener.incoming() {
            thread::spawn(move || {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 8192];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(response.as_bytes());
                }
            });
        }
    });
    addr
}

/// Echoes the request body back as the response body (used by POST body test).
fn start_echo_upstream() -> SocketAddr {
    let (listener, addr) = random_listener();
    thread::spawn(move || {
        for stream in listener.incoming() {
            thread::spawn(move || {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 8192];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let body: Vec<u8> = buf[..n]
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|i| buf[i + 4..n].to_vec())
                        .unwrap_or_default();
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(&body);
                }
            });
        }
    });
    addr
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 1 — Correctness
// ═══════════════════════════════════════════════════════════════════════════════

/// Valid request is forwarded and upstream response is returned to client.
#[test]
fn test_basic_proxy() {
    let upstream_resp = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
    let up = start_mock_upstream(upstream_resp);
    let _p = spawn_proxy(PORT_BASIC, &up.to_string());

    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        up.ip()
    );
    let resp = http_get(PORT_BASIC, &req);
    assert_eq!(status_code(&resp), 200, "expected 200:\n{}", resp);
    assert!(resp.ends_with("hello"), "expected body 'hello':\n{}", resp);
}

/// Request with a Host that does not match the outbound must be rejected 400.
#[test]
fn test_host_rejection() {
    let up = start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let _p = spawn_proxy(PORT_HOST_REJECT, &up.to_string());

    let resp = http_get(
        PORT_HOST_REJECT,
        "GET / HTTP/1.1\r\nHost: evil.com\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(
        status_code(&resp),
        400,
        "wrong Host must return 400:\n{}",
        resp
    );
}

/// First collect_logs returns N lines; second call (no new traffic) returns empty.
#[test]
fn test_collect_logs_drain() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_COLLECT_DRAIN, &up.to_string());

    for _ in 0..3 {
        let req = format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            host
        );
        http_get(PORT_COLLECT_DRAIN, &req);
    }

    let r1 = http_get(PORT_COLLECT_DRAIN, LOG_REQ);
    assert_eq!(status_code(&r1), 200);
    let lines: Vec<&str> = response_body(&r1)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 3, "expected 3 log lines: {:?}", lines);
    for line in &lines {
        let p: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(p.len(), 3, "each line must have 3 fields: {:?}", line);
        assert!(p[0].parse::<u64>().is_ok(), "field 0 must be a timestamp");
        assert!(p[2].parse::<u32>().is_ok(), "field 2 must be a PID");
    }

    let r2 = http_get(PORT_COLLECT_DRAIN, LOG_REQ);
    assert_eq!(status_code(&r2), 200);
    assert!(
        response_body(&r2).trim().is_empty(),
        "second collect_logs must be empty: {:?}",
        response_body(&r2)
    );
}

/// 100 simultaneous connections must all get 200 responses.
#[test]
fn test_concurrent_requests() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_CONCURRENT, &up.to_string());

    let codes = Arc::new(Mutex::new(Vec::<u16>::new()));
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let addr = PORT_CONCURRENT.to_string();
            let h = host.clone();
            let codes = Arc::clone(&codes);
            thread::spawn(move || {
                let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", h);
                codes
                    .lock()
                    .unwrap()
                    .push(status_code(&http_get(&addr, &req)));
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    let c = codes.lock().unwrap();
    assert_eq!(c.len(), 100);
    for &code in c.iter() {
        assert_eq!(code, 200);
    }
}

/// SIGKILL to one worker; supervisor must spawn a replacement within 500 ms.
#[test]
fn test_worker_crash_recovery() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_CRASH, &up.to_string());

    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );
    http_get(PORT_CRASH, &req);
    thread::sleep(Duration::from_millis(100));

    let log = http_get(PORT_CRASH, LOG_REQ);
    let pid: Option<u32> = response_body(&log)
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|s| s.parse().ok());

    if let Some(pid) = pid {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
        thread::sleep(Duration::from_millis(500));
        let resp = http_get(PORT_CRASH, &req);
        assert_eq!(
            status_code(&resp),
            200,
            "proxy must keep serving after worker crash:\n{}",
            resp
        );
    } else {
        eprintln!("test_worker_crash_recovery: no PID logged — skipping kill step");
    }
}

/// Request with no Host header at all must return 400.
#[test]
fn test_missing_host_header_rejected() {
    let up = start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let _p = spawn_proxy(PORT_NO_HOST, &up.to_string());

    let resp = http_get(PORT_NO_HOST, "GET / HTTP/1.1\r\nConnection: close\r\n\r\n");
    assert_eq!(
        status_code(&resp),
        400,
        "missing Host must return 400:\n{}",
        resp
    );
}

/// Host header that includes the port suffix (e.g. "127.0.0.1:9000") must be
/// accepted — the proxy strips the port before comparing.
#[test]
fn test_host_with_port_suffix_accepted() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let _p = spawn_proxy(PORT_HOST_PORT_HDR, &up.to_string());

    // up Displays as "127.0.0.1:PORT" — the port suffix is the point of this test.
    let req = format!("GET / HTTP/1.1\r\nHost: {up}\r\nConnection: close\r\n\r\n");
    let resp = http_get(PORT_HOST_PORT_HDR, &req);
    assert_eq!(
        status_code(&resp),
        200,
        "Host with port suffix must be accepted:\n{}",
        resp
    );
}

/// POST body must arrive at the upstream verbatim (echo upstream verifies this).
#[test]
fn test_post_body_forwarded_verbatim() {
    let up = start_echo_upstream();
    let _p = spawn_proxy(PORT_POST_BODY, &up.to_string());

    let body = "field1=hello&field2=world";
    let req = format!(
        "POST /api HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        up.ip(),
        body.len(),
        body
    );
    let resp = http_get(PORT_POST_BODY, &req);
    assert_eq!(status_code(&resp), 200, "POST must return 200:\n{}", resp);
    assert_eq!(
        response_body(&resp),
        body,
        "upstream must receive the exact POST body"
    );
}

/// collect_logs on a fresh proxy (no traffic yet) returns 200 with empty body.
#[test]
fn test_collect_logs_empty_initially() {
    let up = start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let _p = spawn_proxy(PORT_EMPTY_LOGS, &up.to_string());

    let resp = http_get(PORT_EMPTY_LOGS, LOG_REQ);
    assert_eq!(status_code(&resp), 200);
    assert!(
        response_body(&resp).trim().is_empty(),
        "fresh proxy: collect_logs body must be empty: {:?}",
        response_body(&resp)
    );
}

/// Response larger than one read buffer (64 KB > 8 KB) must be forwarded intact.
#[test]
fn test_large_response_forwarded_intact() {
    const SIZE: usize = 64 * 1024;
    let big = "X".repeat(SIZE);
    let s = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        SIZE, big
    );
    let static_resp: &'static str = Box::leak(s.into_boxed_str());
    let up = start_mock_upstream(static_resp);
    let _p = spawn_proxy(PORT_LARGE_RESP, &up.to_string());

    let req = format!(
        "GET /big HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        up.ip()
    );
    let resp = http_get(PORT_LARGE_RESP, &req);
    assert_eq!(status_code(&resp), 200);
    let body = response_body(&resp);
    assert_eq!(
        body.len(),
        SIZE,
        "large response must be fully forwarded ({} bytes), got {}",
        SIZE,
        body.len()
    );
    assert!(
        body.bytes().all(|b| b == b'X'),
        "large response body must be all 'X'"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 2 — Concurrency Correctness
// ═══════════════════════════════════════════════════════════════════════════════

/// K concurrent collect_logs calls must produce no duplicate entries.
///
/// Why this guarantees correctness:
///   drain_logs() holds the mutex for the ENTIRE drain (read write_pos +
///   copy entries + advance drain_pos is one critical section). At most ONE
///   concurrent caller gets the entries; the others see drain_pos==write_pos
///   and return empty. Sum across K callers must equal N sent.
#[test]
fn test_collect_logs_no_duplicates_concurrent_drain() {
    const N: usize = 150;
    const K: usize = 5; // concurrent drainers

    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_NO_DUPS, &up.to_string());

    // Send N requests sequentially — http_get returns only after log_request
    // is called (log_request runs before the client socket is closed).
    for _ in 0..N {
        let req = format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            host
        );
        assert_eq!(status_code(&http_get(PORT_NO_DUPS, &req)), 200);
    }
    thread::sleep(Duration::from_millis(200)); // safety margin

    let handles: Vec<_> = (0..K)
        .map(|_| thread::spawn(|| count_log_lines(&http_get(PORT_NO_DUPS, LOG_REQ))))
        .collect();
    let total: usize = handles
        .into_iter()
        .map(|h| h.join().expect("drain thread panicked"))
        .sum();

    assert_eq!(
        total, N,
        "sum across {} concurrent drains must be exactly {} (no dups, no missing); got {}",
        K, N, total
    );
}

/// Workers writing log entries while another worker drains must not lose or
/// duplicate entries. The mutex serialises all access.
///
/// Invariant: sum of all drain responses == total requests sent.
#[test]
fn test_collect_logs_no_loss_under_concurrent_write_and_drain() {
    const WRITERS: usize = 20;
    const PER: usize = 10;
    const TOTAL: usize = WRITERS * PER;

    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_CONC_DRAIN, &up.to_string());

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let counts = Arc::new(Mutex::new(Vec::<usize>::new()));
    let counts2 = Arc::clone(&counts);

    // Background drainer: calls collect_logs every 50 ms.
    let drain_handle = thread::spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
            let n = count_log_lines(&http_get(PORT_CONC_DRAIN, LOG_REQ));
            counts2.lock().unwrap().push(n);
            thread::sleep(Duration::from_millis(50));
        }
    });

    // Writer threads.
    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let addr = PORT_CONC_DRAIN.to_string();
            let h = host.clone();
            thread::spawn(move || {
                for _ in 0..PER {
                    let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", h);
                    assert_eq!(status_code(&http_get(&addr, &req)), 200);
                }
            })
        })
        .collect();
    for w in writers {
        w.join().expect("writer panicked");
    }

    stop.store(true, Ordering::Relaxed);
    drain_handle.join().ok();

    // Final drain for any entries written after the last periodic drain.
    thread::sleep(Duration::from_millis(200));
    let final_n = count_log_lines(&http_get(PORT_CONC_DRAIN, LOG_REQ));
    counts.lock().unwrap().push(final_n);

    let grand: usize = counts.lock().unwrap().iter().sum();
    assert_eq!(
        grand, TOTAL,
        "total drained must equal {} requests sent; got {} (possible data race)",
        TOTAL, grand
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 3 — Traffic Distribution
// ═══════════════════════════════════════════════════════════════════════════════

/// Traffic must be spread across multiple workers via SO_REUSEPORT.
/// Each connection has a unique source port; the kernel hashes src(ip,port)
/// to select a worker. On a multi-CPU machine ≥ 2 distinct PIDs must appear.
#[test]
fn test_traffic_distributed_across_workers() {
    let cpus = num_logical_cpus();
    if cpus < 2 {
        eprintln!("traffic_distribution: 1 CPU — only 1 worker, skipping assertion");
        return;
    }

    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_DISTRIBUTION, &up.to_string());

    // 10 threads × 20 requests = 200 connections with different source ports.
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let addr = PORT_DISTRIBUTION.to_string();
            let h = host.clone();
            thread::spawn(move || {
                for _ in 0..20 {
                    let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", h);
                    assert_eq!(status_code(&http_get(&addr, &req)), 200);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    thread::sleep(Duration::from_millis(300));

    let log = http_get(PORT_DISTRIBUTION, LOG_REQ);
    let unique: HashSet<u32> = parse_pids(&log).into_iter().collect();

    eprintln!(
        "[traffic_distribution] {}-CPU system: {} unique worker PIDs across 200 requests: {:?}",
        cpus,
        unique.len(),
        unique
    );

    // Linux: kernel-level SO_REUSEPORT hash distributes connections across workers.
    // macOS: SO_REUSEPORT allows multiple binds but does NOT do kernel load-balancing;
    //        all connections may land on the first-bound socket. Skip the assertion there.
    #[cfg(target_os = "linux")]
    assert!(
        unique.len() >= 2,
        "expected ≥ 2 workers on {}-CPU Linux machine; got {} PID(s): {:?}",
        cpus,
        unique.len(),
        unique
    );

    #[cfg(not(target_os = "linux"))]
    eprintln!(
        "NOTE: macOS SO_REUSEPORT does not guarantee load-balancing across processes. \
         Saw {} unique PID(s). On Linux this would show ≥ 2.",
        unique.len()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 4 — Stress Tests
// ═══════════════════════════════════════════════════════════════════════════════

/// 1 000 sequential requests — zero failures, no hangs, no resource leaks.
#[test]
fn test_stress_1000_sequential() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_STRESS_SEQ, &up.to_string());

    let mut failures = 0usize;
    for i in 0..1000usize {
        let req = format!(
            "GET /{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            i, host
        );
        if status_code(&http_get(PORT_STRESS_SEQ, &req)) != 200 {
            failures += 1;
        }
    }
    assert_eq!(failures, 0, "{}/1000 sequential requests failed", failures);
}

/// 500 concurrent connections — all must receive 200 responses.
#[test]
fn test_stress_500_concurrent() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_STRESS_CONC, &up.to_string());

    let errors = Arc::new(Mutex::new(0usize));
    let handles: Vec<_> = (0..500)
        .map(|_| {
            let addr = PORT_STRESS_CONC.to_string();
            let h = host.clone();
            let e = Arc::clone(&errors);
            thread::spawn(move || {
                let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", h);
                if status_code(&http_get(&addr, &req)) != 200 {
                    *e.lock().unwrap() += 1;
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    let n = *errors.lock().unwrap();
    assert_eq!(n, 0, "{}/500 concurrent requests failed", n);
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 5 — Performance Benchmarks
// ═══════════════════════════════════════════════════════════════════════════════
//
// These tests measure and PRINT performance metrics (req/s, p50, p99).
// They assert correctness (all requests succeed) and a generous latency ceiling
// (p99 < 2 s) to catch hangs, but do NOT assert specific throughput numbers
// because those vary across CI environments.

/// Sequential throughput: reports req/s and p50/p99 latency.
#[test]
fn bench_sequential_throughput() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_BENCH_SEQ, &up.to_string());
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );

    for _ in 0..10 {
        http_get(PORT_BENCH_SEQ, &req);
    } // warm-up

    const N: usize = 200;
    let mut lat = Vec::with_capacity(N);
    let wall = Instant::now();
    for _ in 0..N {
        let (resp, d) = http_get_timed(PORT_BENCH_SEQ, &req);
        assert_eq!(status_code(&resp), 200);
        lat.push(d.as_micros());
    }
    let elapsed = wall.elapsed();
    let rps = N as f64 / elapsed.as_secs_f64();
    let p50 = percentile(lat.clone(), 50);
    let p99 = percentile(lat.clone(), 99);

    eprintln!(
        "[bench_sequential] {} req / {:.2?} → {:.0} req/s | p50={} µs | p99={} µs",
        N, elapsed, rps, p50, p99
    );
    assert!(p99 < 2_000_000, "p99={} µs — suspected hang (> 2 s)", p99);
}

/// Concurrent throughput: 50 threads × 20 requests = 1 000 total.
#[test]
fn bench_concurrent_throughput() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_BENCH_CONC, &up.to_string());
    let warm = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );
    for _ in 0..10 {
        http_get(PORT_BENCH_CONC, &warm);
    }

    const THREADS: usize = 50;
    const PER: usize = 20;
    const TOTAL: usize = THREADS * PER;

    let errors = Arc::new(Mutex::new(0usize));
    let wall = Instant::now();
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let addr = PORT_BENCH_CONC.to_string();
            let h = host.clone();
            let e = Arc::clone(&errors);
            thread::spawn(move || {
                let req = format!("GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", h);
                for _ in 0..PER {
                    if status_code(&http_get(&addr, &req)) != 200 {
                        *e.lock().unwrap() += 1;
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
    let elapsed = wall.elapsed();
    let rps = TOTAL as f64 / elapsed.as_secs_f64();
    let failed = *errors.lock().unwrap();

    eprintln!(
        "[bench_concurrent] {}×{} req={} / {:.2?} → {:.0} req/s | {} failed",
        THREADS, PER, TOTAL, elapsed, rps, failed
    );
    assert_eq!(
        failed, 0,
        "{}/{} concurrent bench requests failed",
        failed, TOTAL
    );
}

/// collect_logs must not materially degrade request latency.
///
/// Method:
///   (a) measure p50/p99 with no concurrent draining (baseline)
///   (b) measure p50/p99 while a background thread drains every 10 ms
///
/// The drain holds the shared mutex for a few microseconds (O(n) memcopy of
/// ring buffer entries). Assertion: load-p99 ≤ max(5 ms, 3 × baseline-p99).
#[test]
fn bench_collect_logs_does_not_degrade_request_latency() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_BENCH_LATENCY, &up.to_string());
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );

    for _ in 0..10 {
        http_get(PORT_BENCH_LATENCY, &req);
    } // warm-up

    const N: usize = 100;

    // (a) Baseline.
    let baseline: Vec<u128> = (0..N)
        .map(|_| {
            let (r, d) = http_get_timed(PORT_BENCH_LATENCY, &req);
            assert_eq!(status_code(&r), 200);
            d.as_micros()
        })
        .collect();
    let base_p50 = percentile(baseline.clone(), 50);
    let base_p99 = percentile(baseline.clone(), 99);

    // (b) With concurrent collect_logs every 10 ms.
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    let drain_handle = thread::spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
            http_get(PORT_BENCH_LATENCY, LOG_REQ);
            thread::sleep(Duration::from_millis(10));
        }
    });

    let under: Vec<u128> = (0..N)
        .map(|_| {
            let (r, d) = http_get_timed(PORT_BENCH_LATENCY, &req);
            assert_eq!(status_code(&r), 200);
            d.as_micros()
        })
        .collect();
    stop.store(true, Ordering::Relaxed);
    drain_handle.join().ok();

    let load_p50 = percentile(under.clone(), 50);
    let load_p99 = percentile(under.clone(), 99);

    eprintln!(
        "[bench_collect_logs_impact] baseline p50={} µs p99={} µs | with-drain p50={} µs p99={} µs",
        base_p50, base_p99, load_p50, load_p99
    );

    // Ceiling: 3× baseline p99, or 5 ms (whichever is larger).
    let ceiling = (3 * base_p99).max(5_000);
    assert!(
        load_p99 <= ceiling,
        "collect_logs degraded p99: baseline={} µs → under-drain={} µs (ceiling={} µs)",
        base_p99,
        load_p99,
        ceiling
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 4 — Regression coverage for fixed bugs
// ═══════════════════════════════════════════════════════════════════════════════

/// Chunked responses (no Content-Length) are forwarded and terminate correctly.
#[test]
fn test_chunked_upstream_response() {
    suite_setup();
    // "Hello, chunked world!" is 0x15 bytes; `\` strips the newline + indent.
    const CHUNKED_RESP: &str = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\
        Connection: close\r\n\r\n15\r\nHello, chunked world!\r\n0\r\n\r\n";

    let up = start_mock_upstream(CHUNKED_RESP);
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_CHUNKED, &up.to_string());

    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );
    let resp = http_get(PORT_CHUNKED, &req);
    assert_eq!(
        status_code(&resp),
        200,
        "chunked response: expected 200:\n{}",
        resp
    );
    assert!(
        resp.contains("Transfer-Encoding: chunked"),
        "chunked header must pass through"
    );
    assert!(
        resp.contains("Hello, chunked world!"),
        "chunked body must pass through:\n{}",
        resp
    );
}

/// A large drain (500 entries) is delivered without truncation, then empties.
#[test]
fn test_collect_logs_many_entries() {
    suite_setup();
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let host = up.ip().to_string();
    let _p = spawn_proxy(PORT_MANY_LOGS, &up.to_string());

    http_get(PORT_MANY_LOGS, LOG_REQ); // drain any pre-existing entries

    const N: usize = 500;
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host
    );
    for _ in 0..N {
        assert_eq!(status_code(&http_get(PORT_MANY_LOGS, &req)), 200);
    }

    let log_resp = http_get(PORT_MANY_LOGS, LOG_REQ);
    assert_eq!(
        status_code(&log_resp),
        200,
        "collect_logs failed:\n{}",
        log_resp
    );
    assert!(
        count_log_lines(&log_resp) >= N,
        "expected >= {} entries, got {} — response was truncated",
        N,
        count_log_lines(&log_resp)
    );

    let log_resp2 = http_get(PORT_MANY_LOGS, LOG_REQ);
    assert!(
        response_body(&log_resp2).trim().is_empty(),
        "second drain must be empty"
    );
}

/// A 256 KB POST body is forwarded intact (request larger than the send buffer).
#[test]
fn test_large_post_body_forwarded() {
    suite_setup();
    const BODY_SIZE: usize = 256 * 1024;

    // Upstream that echoes back the number of body bytes it received.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let (mut buf, mut tmp) = (Vec::new(), [0u8; 4096]);
                let mut content_length: Option<usize> = None;
                let mut header_end = 0usize;
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if content_length.is_none()
                                && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                            {
                                header_end = pos + 4;
                                let headers = std::str::from_utf8(&buf[..header_end]).unwrap_or("");
                                for line in headers.lines() {
                                    if let Some(v) =
                                        line.to_ascii_lowercase().strip_prefix("content-length:")
                                    {
                                        content_length = v.trim().parse().ok();
                                    }
                                }
                            }
                            if let Some(cl) = content_length
                                && buf.len() >= header_end + cl
                            {
                                break;
                            }
                        }
                    }
                }
                let got = buf.len().saturating_sub(header_end);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    got.to_string().len(),
                    got,
                );
                let _ = stream.write_all(resp.as_bytes());
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy(PORT_LARGE_POST, &upstream_addr.to_string());

    let mut request_bytes = format!(
        "POST / HTTP/1.1\r\nHost: {host}\r\nContent-Length: {BODY_SIZE}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    request_bytes.extend(std::iter::repeat_n(b'x', BODY_SIZE));
    let proxy_addr: SocketAddr = PORT_LARGE_POST.parse().unwrap();

    // Only a transient 502 is retried: the upstream connect can momentarily fail
    // when the parallel suite saturates the host. A truncated body would echo the
    // wrong byte count and fail the assertion below without ever retrying.
    let mut resp_str = String::new();
    for _ in 0..3 {
        let mut conn =
            TcpStream::connect_timeout(&proxy_addr, Duration::from_secs(5)).expect("connect");
        conn.set_write_timeout(Some(Duration::from_secs(10))).ok();
        conn.set_read_timeout(Some(Duration::from_secs(10))).ok();
        conn.write_all(&request_bytes).expect("write large POST");
        let mut resp = Vec::new();
        conn.read_to_end(&mut resp).ok();
        resp_str = String::from_utf8_lossy(&resp).into_owned();
        if status_code(&resp_str) == 200 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        status_code(&resp_str),
        200,
        "large POST: expected 200:\n{}",
        resp_str
    );
    assert!(
        resp_str.contains(&BODY_SIZE.to_string()),
        "upstream should have received {} body bytes:\n{}",
        BODY_SIZE,
        resp_str
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// SECTION 5 — Edge-proxy hardening
// ═══════════════════════════════════════════════════════════════════════════════

/// A slow client must still receive a large response in full. This drives the
/// upstream-read backpressure (pause above the high-water mark, resume on drain).
#[test]
fn test_slow_client_receives_full_large_response() {
    const BODY: usize = 2 * 1024 * 1024; // 2 MiB, well past the high-water mark

    // Leak a static body so the mock closure is 'static.
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {BODY}\r\nConnection: close\r\n\r\n");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let header = header.clone();
            thread::spawn(move || {
                let mut tmp = [0u8; 4096];
                let _ = stream.read(&mut tmp); // consume the request
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&vec![b'z'; BODY]);
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy_env(
        PORT_SLOW_CLIENT,
        &upstream_addr.to_string(),
        &[("PROXY_WORKERS", "1")],
    );

    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let mut conn = TcpStream::connect(PORT_SLOW_CLIENT).expect("connect");
    conn.set_read_timeout(Some(Duration::from_secs(30))).ok();
    conn.write_all(req.as_bytes()).unwrap();

    // Read slowly in small chunks so the proxy's response buffer backs up.
    let mut received = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                received.extend_from_slice(&chunk[..n]);
                thread::sleep(Duration::from_micros(200));
            }
            Err(_) => break,
        }
    }

    let body_start = received
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(0);
    assert_eq!(
        received.len() - body_start,
        BODY,
        "slow client must receive the whole body intact"
    );
}

/// When a pooled upstream socket was closed by the peer, the proxy must resend
/// the request on a fresh connection rather than return an empty reply. The mock
/// serves one keep-alive response per connection and then closes, so every reuse
/// after the first hits a stale socket.
#[test]
fn test_stale_pooled_connection_retried() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let mut tmp = [0u8; 4096];
                let _ = stream.read(&mut tmp);
                // keep-alive so the proxy pools the socket, then close it to make
                // that pooled entry stale for the next request.
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok",
                );
                // drop(stream) closes the connection the proxy just pooled
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy_env(
        PORT_STALE_POOL,
        &upstream_addr.to_string(),
        &[("PROXY_WORKERS", "1")],
    );

    // The first request populates the pool; subsequent ones reuse a now-dead
    // socket and must transparently retry.
    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    for i in 0..8 {
        let resp = http_get(PORT_STALE_POOL, &req);
        assert_eq!(
            status_code(&resp),
            200,
            "request {i} got non-200 (stale pool not retried):\n{resp}"
        );
        assert!(resp.ends_with("ok"), "request {i} body truncated:\n{resp}");
    }
}

/// A keep-alive client must be able to send several requests on one connection.
#[test]
fn test_client_keepalive_reuses_connection() {
    let up = start_mock_upstream(
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: keep-alive\r\n\r\nabc",
    );
    let host = up.ip().to_string();
    let _p = spawn_proxy_env(PORT_CLIENT_KA, &up.to_string(), &[("PROXY_WORKERS", "1")]);

    let mut conn = TcpStream::connect(PORT_CLIENT_KA).expect("connect");
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n");

    // Two requests, one connection. Each response is "...\r\n\r\nabc" (3-byte body).
    for n in 0..2 {
        conn.write_all(req.as_bytes()).unwrap();
        let body = read_one_response(&mut conn);
        assert!(
            body.starts_with("HTTP/1.1 200"),
            "request {n} expected 200 on reused conn:\n{body}"
        );
        assert!(body.ends_with("abc"), "request {n} wrong body:\n{body}");
    }
}

/// More headers than the parser accepts must be rejected, not buffered forever.
#[test]
fn test_header_flood_rejected() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _p = spawn_proxy_env(
        PORT_HEADER_FLOOD,
        &up.to_string(),
        &[("PROXY_WORKERS", "1")],
    );

    let mut req = String::from("GET / HTTP/1.1\r\nHost: upstream\r\n");
    for i in 0..200 {
        req.push_str(&format!("X-Flood-{i}: value\r\n"));
    }
    req.push_str("\r\n");
    let resp = http_get(PORT_HEADER_FLOOD, &req);
    assert_eq!(
        status_code(&resp),
        431,
        "header flood must return 431:\n{}",
        resp
    );
}

/// A client that dribbles a request without ever completing the headers must be
/// timed out (slowloris defense), not held open indefinitely.
#[test]
fn test_slowloris_request_times_out() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _p = spawn_proxy_env(
        PORT_SLOWLORIS,
        &up.to_string(),
        &[("PROXY_WORKERS", "1"), ("PROXY_HEADER_MS", "500")],
    );

    let mut conn = TcpStream::connect(PORT_SLOWLORIS).expect("connect");
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    // Partial request: headers never terminated.
    conn.write_all(b"GET / HTTP/1.1\r\nHost: upstream\r\n")
        .unwrap();

    // The proxy should close (or 408) within ~2s given the 500ms header timeout.
    let mut buf = Vec::new();
    let _ = conn.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    assert!(
        buf.is_empty() || text.contains(" 408 "),
        "slowloris connection should be closed or 408, got:\n{}",
        text
    );
}

/// Conflicting Content-Length headers are a smuggling vector and must be rejected.
#[test]
fn test_conflicting_content_length_rejected() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _p = spawn_proxy_env(PORT_SMUGGLE_CL, &up.to_string(), &[("PROXY_WORKERS", "1")]);

    let req = "POST / HTTP/1.1\r\nHost: upstream\r\nContent-Length: 5\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhello";
    let resp = http_get(PORT_SMUGGLE_CL, req);
    assert_eq!(
        status_code(&resp),
        400,
        "conflicting Content-Length must be 400:\n{}",
        resp
    );
}

/// Content-Length together with Transfer-Encoding must be rejected.
#[test]
fn test_content_length_with_transfer_encoding_rejected() {
    let up =
        start_mock_upstream("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _p = spawn_proxy_env(PORT_SMUGGLE_TE, &up.to_string(), &[("PROXY_WORKERS", "1")]);

    let req = "POST / HTTP/1.1\r\nHost: upstream\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n";
    let resp = http_get(PORT_SMUGGLE_TE, req);
    assert_eq!(status_code(&resp), 400, "CL+TE must be 400:\n{}", resp);
}

/// A chunked request body must reach the upstream with its framing intact: the
/// upstream verifies it sees `Transfer-Encoding: chunked` and a decodable body.
#[test]
fn test_chunked_request_body_forwarded() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                // Read until we see the terminal chunk.
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(5).any(|w| w == b"0\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let ok = text.contains("transfer-encoding: chunked")
                    && buf.windows(5).any(|w| w == b"0\r\n\r\n");
                let code = if ok { "200 OK" } else { "400 Bad Request" };
                let _ = stream.write_all(
                    format!("HTTP/1.1 {code}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy_env(
        PORT_CHUNKED_REQ,
        &upstream_addr.to_string(),
        &[("PROXY_WORKERS", "1")],
    );

    // "5\r\nhello\r\n0\r\n\r\n" is a single 5-byte chunk then the terminator.
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {host}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n"
    );
    let mut conn = TcpStream::connect(PORT_CHUNKED_REQ).expect("connect");
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let _ = conn.read_to_end(&mut buf);
    let resp = String::from_utf8_lossy(&buf);
    assert_eq!(
        status_code(&resp),
        200,
        "upstream did not see intact chunked framing:\n{}",
        resp
    );
}

/// Read exactly one HTTP response (headers + Content-Length body) from a stream.
fn read_one_response(conn: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        if header_end.is_none()
            && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
        {
            header_end = Some(pos + 4);
            let headers = String::from_utf8_lossy(&buf[..pos]);
            for line in headers.lines() {
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
        }
        if let Some(he) = header_end
            && buf.len() >= he + content_length
        {
            break;
        }
        match conn.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// A chunked response whose final header value ends in '0' (e.g. max-age=0),
/// with the header section sent in its own write so the proxy parses the headers
/// before any body arrives. The terminal-chunk scan must key off the body, not
/// the "...0\r\n\r\n" that ends those headers, so the body arrives intact.
#[test]
fn test_chunked_response_with_zero_terminated_header() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let mut tmp = [0u8; 4096];
                let _ = stream.read(&mut tmp);
                // Headers first; the last one ends in '0' so the terminator reads
                // "...0\r\n\r\n" — exactly what has_terminal_chunk scans for.
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nCache-Control: max-age=0\r\n\r\n",
                );
                let _ = stream.flush();
                // Body in a later write, so the proxy processes the headers alone.
                thread::sleep(Duration::from_millis(50));
                let _ = stream.write_all(b"5\r\nhello\r\n0\r\n\r\n");
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy(PORT_CHUNKED_HDR_ZERO, &upstream_addr.to_string());

    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let resp = http_get(PORT_CHUNKED_HDR_ZERO, &req);
    assert_eq!(status_code(&resp), 200, "expected 200:\n{}", resp);
    assert!(
        resp.contains("max-age=0"),
        "the zero-terminated header must be forwarded:\n{}",
        resp
    );
    assert!(
        resp.contains("hello"),
        "chunked body must not be truncated by a header false-match:\n{}",
        resp
    );
}

/// A pooled upstream whose peer hard-reset the socket (RST, not a clean FIN)
/// must still recover: whether the reset surfaces on the proxy's write or its
/// first read, the request never reached the upstream and is retried on a fresh
/// connection, so every request returns 200.
#[test]
fn test_stale_pooled_connection_write_error_retried() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            thread::spawn(move || {
                let mut tmp = [0u8; 4096];
                let _ = stream.read(&mut tmp);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok",
                );
                // SO_LINGER 0 makes close() send an RST, so the pooled socket is
                // hard-reset; the proxy's next write to it fails outright.
                let linger = libc::linger {
                    l_onoff: 1,
                    l_linger: 0,
                };
                unsafe {
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        &linger as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::linger>() as libc::socklen_t,
                    );
                }
            });
        }
    });

    let host = upstream_addr.ip().to_string();
    let _p = spawn_proxy_env(
        PORT_STALE_POOL_RST,
        &upstream_addr.to_string(),
        &[("PROXY_WORKERS", "1")],
    );

    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    for i in 0..8 {
        let resp = http_get(PORT_STALE_POOL_RST, &req);
        assert_eq!(
            status_code(&resp),
            200,
            "request {i} got non-200 (reset pool socket not retried):\n{resp}"
        );
        assert!(resp.ends_with("ok"), "request {i} body truncated:\n{resp}");
        // Give the RST time to land so the next reuse fails on write, not read.
        thread::sleep(Duration::from_millis(20));
    }
}
