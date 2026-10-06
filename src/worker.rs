//! Per-worker epoll event loop. One worker = one core = one `mio::Poll`.
//! Connection phases: ReadingRequest → ConnectingUpstream → Forwarding,
//! with CollectLogs as a side path for `/.svc/collect_logs`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::os::unix::io::FromRawFd;
use std::time::{Duration, Instant};

use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token};

use crate::ipc::LogBuffer;
use crate::logs;
use crate::proxy::{self, BodyFraming, ParseStatus};

const LISTENER_TOKEN: Token = Token(0);

// Token packing: id*2 = client socket, id*2+1 = upstream.
fn client_token(id: usize) -> Token {
    Token(id * 2)
}
fn upstream_token(id: usize) -> Token {
    Token(id * 2 + 1)
}
fn conn_id(t: Token) -> usize {
    t.0 / 2
}
fn is_upstream_token(t: Token) -> bool {
    t.0 % 2 == 1
}

const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const KEEPALIVE_MAX_BODY: u64 = 1 << 20;
const RESP_HIGH_WATER: usize = 256 * 1024;
const RESP_LOW_WATER: usize = 64 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_TICK: Duration = Duration::from_secs(1);

/// Connection timeouts, overridable via env for tests.
#[derive(Clone, Copy)]
struct Limits {
    idle: Duration,
    header: Duration,
}

impl Limits {
    fn from_env() -> Self {
        let ms = |key, default: Duration| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_millis)
                .unwrap_or(default)
        };
        Limits {
            idle: ms("PROXY_IDLE_MS", IDLE_TIMEOUT),
            header: ms("PROXY_HEADER_MS", HEADER_TIMEOUT),
        }
    }
}

/// Per-worker idle upstream connections (no lock — workers are single-threaded).
const UPSTREAM_POOL_SIZE: usize = 64;

struct UpstreamPool {
    idle: std::collections::VecDeque<TcpStream>,
}

impl UpstreamPool {
    fn new() -> Self {
        Self {
            idle: std::collections::VecDeque::new(),
        }
    }

    fn take(&mut self) -> Option<TcpStream> {
        self.idle.pop_front()
    }

    fn put(&mut self, stream: TcpStream) {
        if self.idle.len() < UPSTREAM_POOL_SIZE {
            self.idle.push_back(stream);
        }
    }
}

/// Tracks response completion for upstream reuse: Content-Length byte count,
/// or terminal-chunk scan for chunked responses.
struct RespParse {
    header_buf: Vec<u8>,
    headers_done: bool,
    content_length: Option<u64>,
    body_received: u64,
    keep_alive: bool,
    chunked: bool,
    /// Offset of the body within `resp_buf`, adjusted as the buffer drains.
    body_start: usize,
}

impl RespParse {
    fn new() -> Self {
        RespParse {
            header_buf: Vec::new(),
            headers_done: false,
            content_length: None,
            body_received: 0,
            keep_alive: false,
            chunked: false,
            body_start: 0,
        }
    }
}

enum ConnPhase {
    ReadingRequest,
    ConnectingUpstream,
    Forwarding,
    CollectLogs,
}

struct Conn {
    id: usize,
    phase: ConnPhase,
    client: TcpStream,
    client_addr: SocketAddr,
    upstream: Option<TcpStream>,
    req_buf: Vec<u8>,
    req_pending: Vec<u8>,
    resp_buf: Vec<u8>,
    resp_parse: RespParse,
    closing: bool,
    client_keep_alive: bool,
    keepalive_capable: bool,
    head_request: bool,
    retryable: bool,
    from_pool: bool,
    retried: bool,
    upstream_request: Vec<u8>,
    upstream_paused: bool,
    last_activity: Instant,
    req_started: Option<Instant>,
}

impl Conn {
    fn new(id: usize, client: TcpStream, addr: SocketAddr) -> Self {
        Conn {
            id,
            phase: ConnPhase::ReadingRequest,
            client,
            client_addr: addr,
            upstream: None,
            req_buf: Vec::with_capacity(4096),
            req_pending: Vec::new(),
            resp_buf: Vec::new(),
            resp_parse: RespParse::new(),
            closing: false,
            client_keep_alive: false,
            keepalive_capable: false,
            head_request: false,
            retryable: false,
            from_pool: false,
            retried: false,
            upstream_request: Vec::new(),
            upstream_paused: false,
            last_activity: Instant::now(),
            req_started: None,
        }
    }

    fn reset_for_next_request(&mut self) {
        self.phase = ConnPhase::ReadingRequest;
        self.req_buf.clear();
        self.req_pending.clear();
        self.resp_buf.clear();
        self.resp_parse = RespParse::new();
        self.upstream = None;
        self.upstream_request.clear();
        self.closing = false;
        self.client_keep_alive = false;
        self.keepalive_capable = false;
        self.head_request = false;
        self.retryable = false;
        self.from_pool = false;
        self.retried = false;
        self.upstream_paused = false;
        self.req_started = None;
    }

    fn is_expired(&self, now: Instant, limits: &Limits) -> bool {
        if now.duration_since(self.last_activity) > limits.idle {
            return true;
        }
        if matches!(self.phase, ConnPhase::ReadingRequest)
            && let Some(start) = self.req_started
        {
            return now.duration_since(start) > limits.header;
        }
        false
    }
}

/// Owned outcome from `phase_reading` so the borrow on `conn.req_buf` ends
/// before the caller mutates the connection.
enum ReadOutcome {
    Partial,
    Invalid(u16),
    CollectLogs,
    Forward {
        upstream_req: Vec<u8>,
        client_keep_alive: bool,
        keepalive_capable: bool,
        head_request: bool,
        retryable: bool,
    },
}

/// Child entry point after fork(): pin to a core, resolve upstream once,
/// bind a SO_REUSEPORT listener, and run the event loop.
pub fn run_worker(cpu_id: usize, inbound: &str, outbound: &str, log_buf: *mut LogBuffer) -> ! {
    pin_to_cpu(cpu_id);

    let addr = parse_inbound_addr(inbound, cpu_id);

    // Resolve upstream DNS once at startup — getaddrinfo() can block.
    let addr_str = if outbound.contains(':') {
        outbound.to_string()
    } else {
        format!("{}:80", outbound)
    };
    let upstream_addr = resolve_addr(&addr_str).unwrap_or_else(|| {
        eprintln!("worker[{}]: cannot resolve upstream '{}'", cpu_id, addr_str);
        std::process::exit(1);
    });

    let outbound_host: String = outbound.split(':').next().unwrap_or(outbound).to_string();

    // SO_REUSEPORT must be set before bind().
    let std_listener = create_reuseport_listener(addr, cpu_id);
    let mut listener = TcpListener::from_std(std_listener);

    let poll = Poll::new().unwrap_or_else(|e| {
        eprintln!("worker[{}]: Poll::new() failed: {}", cpu_id, e);
        std::process::exit(1);
    });
    poll.registry()
        .register(&mut listener, LISTENER_TOKEN, Interest::READABLE)
        .unwrap_or_else(|e| {
            eprintln!("worker[{}]: register listener failed: {}", cpu_id, e);
            std::process::exit(1);
        });

    eprintln!(
        "worker[{}]: listening on {} → upstream {}",
        cpu_id, addr, upstream_addr
    );
    event_loop(
        cpu_id,
        &outbound_host,
        upstream_addr,
        log_buf,
        poll,
        listener,
        Limits::from_env(),
    );
}

fn event_loop(
    cpu_id: usize,
    outbound_host: &str,
    upstream_addr: SocketAddr,
    log_buf: *mut LogBuffer,
    mut poll: Poll,
    listener: TcpListener,
    limits: Limits,
) -> ! {
    let mut events = Events::with_capacity(1024);
    let mut conns: HashMap<usize, Conn> = HashMap::new();
    let mut next_id: usize = 1;
    let mut pool = UpstreamPool::new();

    loop {
        if let Err(e) = poll.poll(&mut events, Some(POLL_TICK)) {
            if e.kind() != io::ErrorKind::Interrupted {
                eprintln!("worker[{}]: poll error: {}", cpu_id, e);
            }
            continue;
        }

        for event in events.iter() {
            let token = event.token();
            if token == LISTENER_TOKEN {
                accept_loop(poll.registry(), &listener, &mut conns, &mut next_id, cpu_id);
                continue;
            }

            let id = conn_id(token);
            if let Some(mut conn) = conns.remove(&id) {
                conn.last_activity = Instant::now();
                let keep = dispatch(
                    poll.registry(),
                    &mut conn,
                    event,
                    outbound_host,
                    upstream_addr,
                    log_buf,
                    &mut pool,
                    cpu_id,
                );
                if keep {
                    conns.insert(id, conn);
                }
            }
        }

        sweep_timeouts(poll.registry(), &mut conns, &limits);
    }
}

fn sweep_timeouts(registry: &Registry, conns: &mut HashMap<usize, Conn>, limits: &Limits) {
    let now = Instant::now();
    let expired: Vec<usize> = conns
        .iter()
        .filter(|(_, c)| c.is_expired(now, limits))
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some(mut conn) = conns.remove(&id) {
            if matches!(conn.phase, ConnPhase::ReadingRequest) && conn.req_started.is_some() {
                send_error_response(&mut conn.client, 408);
            }
            let _ = registry.deregister(&mut conn.client);
        }
    }
}

fn accept_loop(
    registry: &Registry,
    listener: &TcpListener,
    conns: &mut HashMap<usize, Conn>,
    next_id: &mut usize,
    cpu_id: usize,
) {
    loop {
        match listener.accept() {
            Ok((mut stream, addr)) => {
                let id = *next_id;
                *next_id = next_id.wrapping_add(1);
                if *next_id == 0 {
                    *next_id = 1;
                }
                let _ = registry.register(&mut stream, client_token(id), Interest::READABLE);
                conns.insert(id, Conn::new(id, stream, addr));
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => {
                eprintln!("worker[{}]: accept error: {}", cpu_id, e);
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    registry: &Registry,
    conn: &mut Conn,
    event: &mio::event::Event,
    outbound_host: &str,
    upstream_addr: SocketAddr,
    log_buf: *mut LogBuffer,
    pool: &mut UpstreamPool,
    cpu_id: usize,
) -> bool {
    let upstream_side = is_upstream_token(event.token());

    if !upstream_side && !conn.resp_buf.is_empty() {
        if !flush_resp_to_client(registry, conn) {
            return false;
        }
        if conn.resp_buf.is_empty() && conn.closing {
            return false;
        }
    }

    // Edge-triggered: re-drive a paused upstream read once the client drains.
    if !upstream_side
        && conn.upstream_paused
        && conn.resp_buf.len() <= RESP_LOW_WATER
        && matches!(conn.phase, ConnPhase::Forwarding)
    {
        conn.upstream_paused = false;
        return phase_upstream_read(registry, conn, log_buf, pool, upstream_addr, cpu_id);
    }

    match conn.phase {
        ConnPhase::ReadingRequest => {
            phase_reading(registry, conn, outbound_host, upstream_addr, pool, cpu_id)
        }
        ConnPhase::ConnectingUpstream => phase_connecting(registry, conn, upstream_addr, cpu_id),
        ConnPhase::Forwarding if upstream_side => {
            phase_upstream_read(registry, conn, log_buf, pool, upstream_addr, cpu_id)
        }
        ConnPhase::Forwarding => phase_client_read(registry, conn, cpu_id),
        ConnPhase::CollectLogs => phase_collect_logs(registry, conn, log_buf),
    }
}

fn phase_reading(
    registry: &Registry,
    conn: &mut Conn,
    outbound_host: &str,
    upstream_addr: SocketAddr,
    pool: &mut UpstreamPool,
    cpu_id: usize,
) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match conn.client.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => {
                if conn.req_started.is_none() {
                    conn.req_started = Some(Instant::now());
                }
                conn.req_buf.extend_from_slice(&buf[..n]);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => return false,
        }
    }

    let outcome = parse_reading(conn, outbound_host);
    match outcome {
        ReadOutcome::Partial => {
            if conn.req_buf.len() > MAX_HEADER_BYTES {
                send_error_response(&mut conn.client, 431);
                return false;
            }
            true
        }
        ReadOutcome::Invalid(code) => {
            send_error_response(&mut conn.client, code);
            false
        }
        ReadOutcome::CollectLogs => {
            conn.phase = ConnPhase::CollectLogs;
            let _ =
                registry.reregister(&mut conn.client, client_token(conn.id), Interest::WRITABLE);
            true
        }
        ReadOutcome::Forward {
            upstream_req,
            client_keep_alive,
            keepalive_capable,
            head_request,
            retryable,
        } => {
            conn.req_pending = upstream_req.clone();
            conn.upstream_request = upstream_req;
            conn.client_keep_alive = client_keep_alive;
            conn.keepalive_capable = keepalive_capable;
            conn.head_request = head_request;
            conn.retryable = retryable;
            conn.resp_parse = RespParse::new();
            conn.req_buf.clear();
            connect_to_upstream(registry, conn, upstream_addr, pool, cpu_id)
        }
    }
}

fn parse_reading(conn: &Conn, outbound_host: &str) -> ReadOutcome {
    let mut header_store = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let parsed = match proxy::parse_request(&conn.req_buf, &mut header_store) {
        ParseStatus::Partial => return ReadOutcome::Partial,
        ParseStatus::Invalid(code) => return ReadOutcome::Invalid(code),
        ParseStatus::Complete(p) => p,
    };

    if parsed.path == "/.svc/collect_logs" {
        return ReadOutcome::CollectLogs;
    }
    if !proxy::validate_host(&parsed.headers, outbound_host) {
        return ReadOutcome::Invalid(400);
    }

    let framing = match proxy::request_body_framing(&parsed.headers) {
        Ok(f) => f,
        Err(code) => return ReadOutcome::Invalid(code),
    };

    let header_end = parsed.body_offset;
    let (send_end, body_complete) = match framing {
        BodyFraming::None => (header_end, true),
        BodyFraming::ContentLength(n) => {
            let want = header_end + n as usize;
            if conn.req_buf.len() >= want {
                (want, true)
            } else {
                (conn.req_buf.len(), false)
            }
        }
        BodyFraming::Chunked => (conn.req_buf.len(), false),
    };

    let upstream_req = proxy::build_upstream_request(&parsed, &conn.req_buf[header_end..send_end]);
    let pipelined = body_complete && conn.req_buf.len() > send_end;
    let small_body = matches!(framing, BodyFraming::ContentLength(n) if n <= KEEPALIVE_MAX_BODY)
        || matches!(framing, BodyFraming::None);
    let client_keep_alive = proxy::request_wants_keep_alive(&parsed.headers, parsed.version);

    ReadOutcome::Forward {
        upstream_req,
        client_keep_alive,
        keepalive_capable: client_keep_alive && body_complete && !pipelined && small_body,
        head_request: parsed.method.eq_ignore_ascii_case("HEAD"),
        retryable: body_complete,
    }
}

fn connect_to_upstream(
    registry: &Registry,
    conn: &mut Conn,
    upstream_addr: SocketAddr,
    pool: &mut UpstreamPool,
    cpu_id: usize,
) -> bool {
    if let Some(mut upstream) = pool.take()
        && registry
            .register(&mut upstream, upstream_token(conn.id), Interest::WRITABLE)
            .is_ok()
    {
        conn.upstream = Some(upstream);
        conn.from_pool = true;
        conn.phase = ConnPhase::ConnectingUpstream;
        return true;
    }

    match connect_upstream(upstream_addr) {
        Err(e) => {
            eprintln!(
                "worker[{}]: connect {} failed: {}",
                cpu_id, upstream_addr, e
            );
            send_error_response(&mut conn.client, 502);
            false
        }
        Ok(mut upstream) => {
            let _ = registry.register(&mut upstream, upstream_token(conn.id), Interest::WRITABLE);
            conn.upstream = Some(upstream);
            conn.from_pool = false;
            conn.phase = ConnPhase::ConnectingUpstream;
            true
        }
    }
}

fn phase_connecting(
    registry: &Registry,
    conn: &mut Conn,
    upstream_addr: SocketAddr,
    cpu_id: usize,
) -> bool {
    // WRITABLE means connect() finished. Loop the write for requests larger
    // than the send buffer; on WouldBlock stay in this phase.
    let mut send_failed = None;
    while !conn.req_pending.is_empty() {
        let upstream = match conn.upstream.as_mut() {
            Some(u) => u,
            None => return false,
        };
        match upstream.write(&conn.req_pending) {
            Ok(0) => break,
            Ok(n) => {
                conn.req_pending.drain(..n);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => {
                send_failed = Some(e);
                break;
            }
        }
    }

    if let Some(e) = send_failed {
        // A stale pooled socket can surface RST on write. Retry once on a
        // fresh connection — same recovery as the read-EOF path.
        if conn.from_pool && conn.retryable && !conn.retried {
            return retry_on_fresh_upstream(registry, conn, upstream_addr, cpu_id);
        }
        eprintln!("worker[{}]: upstream send failed: {}", cpu_id, e);
        send_error_response(&mut conn.client, 502);
        return false;
    }

    let interest = if conn.req_pending.is_empty() {
        conn.phase = ConnPhase::Forwarding;
        Interest::READABLE
    } else {
        Interest::WRITABLE
    };
    if let Some(up) = conn.upstream.as_mut() {
        let _ = registry.reregister(up, upstream_token(conn.id), interest);
    }
    true
}

fn phase_upstream_read(
    registry: &Registry,
    conn: &mut Conn,
    log_buf: *mut LogBuffer,
    pool: &mut UpstreamPool,
    upstream_addr: SocketAddr,
    cpu_id: usize,
) -> bool {
    let mut buf = [0u8; 8192];
    let mut closed = false;

    loop {
        let mut hit_high_water = false;
        loop {
            let upstream = match conn.upstream.as_mut() {
                Some(u) => u,
                None => return false,
            };
            match upstream.read(&mut buf) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(n) => {
                    ingest_upstream_bytes(conn, &buf[..n]);
                    if conn.resp_buf.len() >= RESP_HIGH_WATER {
                        hit_high_water = true;
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }

        // Pooled socket already closed by peer: EOF before any response.
        // Resend once on a fresh connection.
        if closed
            && !conn.resp_parse.headers_done
            && conn.resp_buf.is_empty()
            && conn.from_pool
            && conn.retryable
            && !conn.retried
        {
            return retry_on_fresh_upstream(registry, conn, upstream_addr, cpu_id);
        }

        if !conn.resp_buf.is_empty() && !flush_resp_to_client(registry, conn) {
            return false;
        }

        if hit_high_water {
            if conn.resp_buf.len() > RESP_LOW_WATER {
                conn.upstream_paused = true;
                return true;
            }
            continue;
        }
        break;
    }
    conn.upstream_paused = false;

    let headers_done = conn.resp_parse.headers_done;
    let head_done = headers_done && conn.head_request;
    let cl_done = headers_done
        && !conn.head_request
        && conn
            .resp_parse
            .content_length
            .map(|cl| conn.resp_parse.body_received >= cl)
            .unwrap_or(false);
    let chunked_done = !conn.head_request
        && conn.resp_parse.chunked
        && conn.resp_parse.content_length.is_none()
        && conn
            .resp_buf
            .get(conn.resp_parse.body_start..)
            .map(proxy::has_terminal_chunk)
            .unwrap_or(false);
    let self_delimited = head_done || cl_done || chunked_done;

    if !(self_delimited || closed) {
        return true;
    }

    let ip = conn.client_addr.ip().to_string();
    // SAFETY: log_buf is the shared mapping initialized before fork().
    unsafe {
        logs::log_request(log_buf, &ip, std::process::id());
    }

    if (cl_done || head_done)
        && !closed
        && conn.resp_parse.keep_alive
        && !conn.resp_parse.chunked
        && let Some(mut upstream) = conn.upstream.take()
    {
        let _ = registry.deregister(&mut upstream);
        pool.put(upstream);
    }
    if let Some(mut upstream) = conn.upstream.take() {
        let _ = registry.deregister(&mut upstream);
    }

    if self_delimited
        && conn.keepalive_capable
        && conn.resp_parse.keep_alive
        && conn.resp_buf.is_empty()
    {
        conn.reset_for_next_request();
        let _ = registry.reregister(&mut conn.client, client_token(conn.id), Interest::READABLE);
        return true;
    }

    if !conn.resp_buf.is_empty() {
        conn.closing = true;
        return true;
    }
    false
}

fn ingest_upstream_bytes(conn: &mut Conn, bytes: &[u8]) {
    if conn.resp_parse.headers_done {
        conn.resp_buf.extend_from_slice(bytes);
        conn.resp_parse.body_received += bytes.len() as u64;
        return;
    }

    conn.resp_parse.header_buf.extend_from_slice(bytes);
    if let Some(end) = proxy::find_header_end(&conn.resp_parse.header_buf) {
        let hdr = &conn.resp_parse.header_buf[..end];
        conn.resp_parse.content_length = proxy::parse_response_content_length(hdr);
        conn.resp_parse.keep_alive = proxy::response_is_keep_alive(hdr);
        conn.resp_parse.chunked = proxy::response_is_chunked(hdr);
        conn.resp_parse.body_received = (conn.resp_parse.header_buf.len() - end) as u64;
        conn.resp_parse.headers_done = true;
        let head = std::mem::take(&mut conn.resp_parse.header_buf);
        // Body offset so the terminal-chunk scan never matches inside headers.
        conn.resp_parse.body_start = conn.resp_buf.len() + end;
        conn.resp_buf.extend_from_slice(&head);
    } else if conn.resp_parse.header_buf.len() > MAX_HEADER_BYTES {
        let head = std::mem::take(&mut conn.resp_parse.header_buf);
        conn.resp_buf.extend_from_slice(&head);
        conn.resp_parse.headers_done = true;
    }
}

fn retry_on_fresh_upstream(
    registry: &Registry,
    conn: &mut Conn,
    upstream_addr: SocketAddr,
    cpu_id: usize,
) -> bool {
    conn.retried = true;
    conn.from_pool = false;
    if let Some(mut dead) = conn.upstream.take() {
        let _ = registry.deregister(&mut dead);
    }
    match connect_upstream(upstream_addr) {
        Ok(mut upstream) => {
            let _ = registry.register(&mut upstream, upstream_token(conn.id), Interest::WRITABLE);
            conn.upstream = Some(upstream);
            conn.req_pending = conn.upstream_request.clone();
            conn.resp_parse = RespParse::new();
            conn.phase = ConnPhase::ConnectingUpstream;
            true
        }
        Err(e) => {
            eprintln!("worker[{}]: retry connect failed: {}", cpu_id, e);
            send_error_response(&mut conn.client, 502);
            false
        }
    }
}

fn phase_client_read(registry: &Registry, conn: &mut Conn, cpu_id: usize) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match conn.client.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => conn.req_pending.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => return false,
        }
    }

    if let Some(ref mut up) = conn.upstream {
        match up.write(&conn.req_pending) {
            Ok(n) => {
                conn.req_pending.drain(..n);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                let _ = registry.reregister(
                    up,
                    upstream_token(conn.id),
                    Interest::WRITABLE | Interest::READABLE,
                );
            }
            Err(e) => {
                eprintln!("worker[{}]: upstream write: {}", cpu_id, e);
                return false;
            }
        }
    }
    true
}

fn phase_collect_logs(registry: &Registry, conn: &mut Conn, log_buf: *mut LogBuffer) -> bool {
    // Route through resp_buf so large drains survive WouldBlock.
    let lines = unsafe { logs::drain_logs(log_buf) };
    let body = lines.join("\n");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body,
    );
    conn.resp_buf.extend_from_slice(response.as_bytes());
    conn.closing = true;
    flush_resp_to_client(registry, conn) && !conn.resp_buf.is_empty()
}

/// Write `conn.resp_buf` to the client until empty or `WouldBlock`.
/// Returns `false` only on an unrecoverable socket error.
fn flush_resp_to_client(registry: &Registry, conn: &mut Conn) -> bool {
    while !conn.resp_buf.is_empty() {
        match conn.client.write(&conn.resp_buf) {
            Ok(0) => return false,
            Ok(n) => {
                conn.resp_buf.drain(..n);
                conn.resp_parse.body_start = conn.resp_parse.body_start.saturating_sub(n);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                let _ = registry.reregister(
                    &mut conn.client,
                    client_token(conn.id),
                    Interest::READABLE | Interest::WRITABLE,
                );
                return true;
            }
            Err(_) => return false,
        }
    }
    let _ = registry.reregister(&mut conn.client, client_token(conn.id), Interest::READABLE);
    true
}

fn send_error_response(client: &mut TcpStream, code: u16) {
    let (reason, body) = match code {
        400 => ("Bad Request", "Bad Request"),
        408 => ("Request Timeout", "Request Timeout"),
        431 => (
            "Request Header Fields Too Large",
            "Request Header Fields Too Large",
        ),
        502 => ("Bad Gateway", "Bad Gateway"),
        _ => ("Internal Server Error", "Internal Server Error"),
    };
    let msg = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        len = body.len(),
    );
    let _ = client.write_all(msg.as_bytes());
    use std::os::unix::io::AsRawFd;
    // SAFETY: client owns a valid fd; SHUT_WR only sends a FIN.
    unsafe {
        libc::shutdown(client.as_raw_fd(), libc::SHUT_WR);
    }
}

/// Non-blocking upstream connect with SO_REUSEADDR and TCP_NODELAY set before
/// connect. mio's `TcpStream::connect` gives no pre-connect sockopt hook.
fn connect_upstream(addr: SocketAddr) -> io::Result<TcpStream> {
    use std::os::unix::io::FromRawFd;
    let SocketAddr::V4(v4) = addr else {
        return Err(io::Error::other("IPv6 not supported"));
    };

    // SAFETY: every libc call is checked; the fd is closed on any error path.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let one: libc::c_int = 1;
        let opt_sz = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const _ as *const libc::c_void,
            opt_sz,
        );
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &one as *const _ as *const libc::c_void,
            opt_sz,
        );
        libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);

        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = v4.port().to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());

        let ret = libc::connect(
            fd,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINPROGRESS) {
                libc::close(fd);
                return Err(err);
            }
        }

        Ok(TcpStream::from_std(std::net::TcpStream::from_raw_fd(fd)))
    }
}

fn parse_inbound_addr(inbound: &str, cpu_id: usize) -> SocketAddr {
    inbound.parse().unwrap_or_else(|_| {
        format!("{}:80", inbound).parse().unwrap_or_else(|_| {
            eprintln!(
                "worker[{}]: cannot parse inbound address: {}",
                cpu_id, inbound
            );
            std::process::exit(1);
        })
    })
}

fn resolve_addr(addr_str: &str) -> Option<SocketAddr> {
    if let Ok(a) = addr_str.parse::<SocketAddr>() {
        return Some(a);
    }
    use std::net::ToSocketAddrs;
    addr_str.to_socket_addrs().ok()?.next()
}

/// Non-blocking listener with SO_REUSEPORT set before bind().
fn create_reuseport_listener(addr: SocketAddr, cpu_id: usize) -> std::net::TcpListener {
    let ip_port = match addr {
        SocketAddr::V4(v4) => (v4.ip().octets(), v4.port()),
        SocketAddr::V6(_) => {
            eprintln!("worker[{}]: IPv6 not supported", cpu_id);
            std::process::exit(1);
        }
    };

    // SAFETY: every libc call is checked; the process exits on any failure.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            eprintln!(
                "worker[{}]: socket() failed: {}",
                cpu_id,
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }

        let one: libc::c_int = 1;
        let opt_size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const _ as *const libc::c_void,
            opt_size,
        );
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &one as *const _ as *const libc::c_void,
            opt_size,
        );
        libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);

        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = ip_port.1.to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(ip_port.0);

        let ret = libc::bind(
            fd,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        if ret < 0 {
            eprintln!(
                "worker[{}]: bind({}) failed: {}",
                cpu_id,
                addr,
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        if libc::listen(fd, 128) < 0 {
            eprintln!(
                "worker[{}]: listen() failed: {}",
                cpu_id,
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        std::net::TcpListener::from_raw_fd(fd)
    }
}

fn pin_to_cpu(cpu_id: usize) {
    #[cfg(target_os = "linux")]
    // SAFETY: cpu_set_t is plain data; sched_setaffinity only reads it.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu_id, &mut set);
        let ret = libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
        if ret != 0 {
            eprintln!(
                "worker[{}]: sched_setaffinity failed (non-fatal): {}",
                cpu_id,
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cpu_id;
    }
}
