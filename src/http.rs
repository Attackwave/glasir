//! MCP over Streamable HTTP, for agents that are not a local subprocess.
//!
//! One endpoint answering POST and GET, per the current transport spec. The
//! plan calls for "SSE/HTTP", which was the 2024-11-05 transport with a
//! separate SSE endpoint; that has since been superseded, so this implements
//! the replacement. SSE still appears where it belongs: a GET opens a stream
//! the server could push on.
//!
//! No HTTP framework. One endpoint, four headers and a content-length body is
//! less code than wiring up a router, and `mcp::handle` already does the
//! protocol work — this is only a second way to reach it.
//!
//! **Security is not optional here**, and the spec says so: a local server
//! reachable from a browser is a DNS-rebinding target. Three defences, all
//! enforced below: bind to loopback unless told otherwise, validate `Origin`,
//! and require a bearer token when one is configured.

use crate::mcp::{self, ServedState};
use crate::published::Published;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Connections served at once. A query costs microseconds and the state is
/// read-only, so the cap is only there to keep a flood of connections from
/// spawning threads without bound — ten simultaneous clients is the case this
/// is built for, not ten thousand.
const MAX_CONNECTIONS: usize = 32;

/// How long a connection may take to send its request, and to receive its
/// answer.
///
/// Without this a client that opens a socket, sends one byte and then says
/// nothing holds a thread forever: `read_line` blocks with no deadline.
/// Measured against this server before the timeout existed — forty such
/// connections filled every slot and a legitimate request could no longer get a
/// complete answer. A local tool answering in microseconds has no legitimate
/// use for a ten-second request.
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Set only by the minimal async-signal-safe handler below. A listener checks
/// it between accepts, then drains work already admitted before returning.
static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn request_shutdown(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_shutdown_handlers() -> std::io::Result<()> {
    // The handler does only an atomic store, which is async-signal-safe. The
    // server does all logging, waiting and cleanup back on its normal thread.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request_shutdown as *const () as libc::sighandler_t;
        if libc::sigemptyset(&mut action.sa_mask) != 0
            || libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) != 0
            || libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) != 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
extern "system" fn request_console_shutdown(_: u32) -> i32 {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
    1
}

#[cfg(windows)]
fn install_shutdown_handlers() -> std::io::Result<()> {
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    // Windows services and console hosts deliver CTRL_C, close, logoff and
    // shutdown notifications through this callback. It mirrors the Unix
    // handler: only set an atomic flag here; drain from normal Rust code.
    let installed = unsafe { SetConsoleCtrlHandler(Some(request_console_shutdown), 1) };
    if installed == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn install_shutdown_handlers() -> std::io::Result<()> {
    Ok(())
}

/// Largest request body accepted, before reading a byte of it.
///
/// `Content-Length` is attacker-controlled and arrives *before* any
/// authentication, so allocating what it claims hands an anonymous client the
/// process's memory. Linux over-commits, so the naive test looks harmless — the
/// allocation only faults in as it is written — but a filesystem-backed or
/// hardened allocator is under no such obligation, and a 1 MiB cap is far
/// beyond any real MCP request.
const MAX_BODY: usize = 1024 * 1024;

/// Decrements the live-connection count however the thread leaves.
///
/// A bare `fetch_sub` at the end of the closure is skipped by a panic, and the
/// count then never falls: after `MAX_CONNECTIONS` panics the server refuses
/// every connection while still running, which is invisible from the outside.
struct LiveGuard(Arc<AtomicUsize>);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct HttpConfig {
    pub addr: String,
    /// Required in `Authorization: Bearer <token>` when set. A single shared
    /// secret: right for one machine, too coarse for a team — it cannot be
    /// attributed, revoked or expired. `tokens` is what a team uses.
    pub token: Option<String>,
    /// Per-user tokens, re-read when the file changes.
    pub tokens: Arc<crate::auth::Tokens>,
    /// Counters an operator scrapes. Always present, unlike `audit`: a metric
    /// carries no identity and no question, so there is nothing here that a
    /// single-machine server has a reason to switch off.
    pub metrics: Arc<Metrics>,
    /// Where answered requests are recorded, when identities exist to record.
    /// `None` on a single-machine server: without tokens there is no identity,
    /// and a log full of "anonymous" answers no audit question while still
    /// collecting what a developer asked on their own machine.
    pub audit: Option<Arc<crate::audit::Audit>>,
    /// The public name of this instance, when it differs from the bound
    /// address — `--public-url https://glasir.example.com`. See `origin_of`
    /// for why neither the address nor the `Host` field can supply it.
    pub public_url: Option<String>,
    /// Bumped by whoever republishes the served graph, so an open SSE stream
    /// can tell that its client's cached answers describe a tree that has
    /// moved on. A counter rather than a subscriber registry: the streams poll
    /// one shared number, so a re-index costs one atomic store no matter how
    /// many clients are attached, and a client that connects late still sees
    /// that it missed something because the number is higher than the one it
    /// started from. `None` when nothing republishes — a server without
    /// `--watch` answers from the tree as it was at startup, and a stream that
    /// can never fire should not pretend to be live.
    pub reindexed: Option<Arc<AtomicU64>>,
    /// Certificate and key, when the operator supplied them. `None` serves
    /// plain HTTP, which is right for loopback and wrong for anything else.
    pub tls: Option<Arc<rustls::ServerConfig>>,
    /// This process is the private data plane behind `glasir-control`. It
    /// accepts only loopback traffic authenticated with a revocable service
    /// token; public authentication and authorization stay at the edge.
    pub behind_control_plane: bool,
    /// Explicit IPv4 network from which a remote control plane may connect.
    /// Absent means private data-plane mode remains loopback-only.
    pub control_plane_cidr: Option<ControlPlaneCidr>,
    /// Most recent background re-index failure. Readiness reports it without
    /// withdrawing an already valid graph from service.
    pub reindex_error: Arc<Mutex<Option<String>>>,
    /// Optional readiness SLO. A loaded but over-age graph stays available to
    /// callers, while `/ready` returns 503 so an orchestrator can stop routing
    /// new work until indexing catches up.
    pub max_index_age: Option<std::time::Duration>,
}

/// A deliberately small CIDR parser for the one trust boundary the Core owns.
/// IPv6 is rejected rather than silently broadened; operators can add IPv6
/// support only alongside equivalent Kubernetes network-policy coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlPlaneCidr {
    network: u32,
    prefix: u8,
}

impl ControlPlaneCidr {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let (address, prefix) = input
            .rsplit_once('/')
            .ok_or("expected IPv4 CIDR, e.g. 10.1.0.0/16")?;
        let address: Ipv4Addr = address.parse().map_err(|_| "CIDR address must be IPv4")?;
        let prefix: u8 = prefix
            .parse()
            .map_err(|_| "CIDR prefix must be 0 through 32")?;
        if prefix > 32 {
            return Err("CIDR prefix must be 0 through 32");
        }
        let raw = u32::from(address);
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        if raw & !mask != 0 {
            return Err("CIDR address must be the network address (host bits must be zero)");
        }
        Ok(Self {
            network: raw,
            prefix,
        })
    }

    fn contains(&self, address: IpAddr) -> bool {
        let IpAddr::V4(address) = address else {
            return false;
        };
        let mask = if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix)
        };
        u32::from(address) & mask == self.network
    }
}

/// Builds a TLS configuration from a PEM certificate chain and private key.
///
/// No client certificates and no ALPN: the credential here is the bearer token
/// the transport carries, and HTTP/1.1 is what this server speaks. What TLS
/// adds is that the token is not readable on the wire — which is the whole
/// reason the README had to say "terminate it at a reverse proxy".
pub fn tls_config(
    cert: &std::path::Path,
    key: &std::path::Path,
) -> std::io::Result<rustls::ServerConfig> {
    let certs = crate::pem::certs(&mut BufReader::new(std::fs::File::open(cert)?))
        .map_err(|e| std::io::Error::other(format!("{}: {e}", cert.display())))?;
    let key = crate::pem::private_key(&mut BufReader::new(std::fs::File::open(key)?))
        .map_err(|e| std::io::Error::other(format!("{}: {e}", key.display())))?;
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(std::io::Error::other)
}

/// Builds a private data-plane TLS listener which requires and verifies a
/// client certificate. The service token and CIDR gate remain independent.
pub fn mtls_config(
    cert: &std::path::Path,
    key: &std::path::Path,
    client_ca: &std::path::Path,
) -> std::io::Result<rustls::ServerConfig> {
    let certs = crate::pem::certs(&mut BufReader::new(std::fs::File::open(cert)?))?;
    let key = crate::pem::private_key(&mut BufReader::new(std::fs::File::open(key)?))?;
    let client_certs = crate::pem::certs(&mut BufReader::new(std::fs::File::open(client_ca)?))?;
    let mut roots = rustls::RootCertStore::empty();
    for certificate in client_certs {
        roots.add(certificate).map_err(std::io::Error::other)?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(std::io::Error::other)?;
    rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(std::io::Error::other)
}

/// Serves until the listener is dropped.
///
/// One thread per connection, capped at `MAX_CONNECTIONS`. The state is behind
/// an `ArcSwap` so a re-index can replace it while clients are reading: each
/// request loads the current one and holds it for the length of that request,
/// which is the only state a connection has. Nothing is remembered between
/// requests, so a client may connect to any thread and get the same answer.
pub fn serve(state: &Arc<Published<ServedState>>, cfg: &HttpConfig) -> std::io::Result<()> {
    if refuses_to_start(cfg) {
        // Printed rather than returned: an `io::Error` reaches the user through
        // `Debug`, which wraps the advice in escaped quotes and hides the one
        // line that says what to do about it.
        eprintln!(
            "refusing to serve {} with no authentication.\n\
             Anyone who can reach this port could read the graph of your source.\n\
             Issue a token with `glasir token add <name>`, or pass --token <secret>.",
            cfg.addr
        );
        std::process::exit(2);
    }
    // A token on the wire in clear is readable by anything between the client
    // and here. Refusing would break the reverse-proxy deployment the README
    // recommends, where TLS ends one hop earlier — so this warns and serves.
    if !is_loopback(&cfg.addr) && cfg.tls.is_none() {
        eprintln!(
            "glasir: warning — serving {} without TLS. Tokens travel in clear \
unless a proxy terminates TLS in front of this process. Pass --tls-cert and \
--tls-key to terminate it here.",
            cfg.addr
        );
    }
    let listener = TcpListener::bind(&cfg.addr)?;
    install_shutdown_handlers()?;
    listener.set_nonblocking(true)?;
    eprintln!(
        "glasir: MCP over HTTP{} on http{}://{}/mcp",
        if cfg.tls.is_some() { "S" } else { "" },
        if cfg.tls.is_some() { "s" } else { "" },
        cfg.addr
    );
    if cfg.tokens.configured() {
        eprintln!("glasir: per-user tokens in effect");
    }

    let live = Arc::new(AtomicUsize::new(0));
    while !SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
        let (stream, peer) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(25));
                continue;
            }
            Err(e) => return Err(e),
        };
        if cfg.behind_control_plane
            && !is_loopback(&cfg.addr)
            && !cfg
                .control_plane_cidr
                .as_ref()
                .is_some_and(|cidr| cidr.contains(peer.ip()))
        {
            // No HTTP response: an untrusted peer must not learn that a graph
            // service exists, even before it has supplied a service token.
            continue;
        }
        // Refuse rather than queue: a client that waits behind a full server
        // cannot tell that from a hung one, and this is a local tool.
        if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            let mut stream = stream;
            let _ = respond(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                "too many connections",
            );
            continue;
        }
        live.fetch_add(1, Ordering::Relaxed);
        let (state, cfg, live) = (state.clone(), cfg.clone(), live.clone());
        std::thread::spawn(move || {
            let _guard = LiveGuard(live);
            // A client that stops talking must not hold the slot forever.
            let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
            let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
            // One bad request must not take the server down with it.
            let result = match &cfg.tls {
                // The handshake happens on this thread, inside the timeouts and
                // the connection cap, so a client that opens a socket and never
                // completes it is bounded exactly like a plaintext one.
                Some(tls) => match rustls::ServerConnection::new(tls.clone()) {
                    Ok(conn) => {
                        let mut tls_stream = rustls::StreamOwned::new(conn, stream);
                        handle_connection(&state, &cfg, &mut tls_stream)
                    }
                    Err(e) => {
                        eprintln!("http: tls: {e}");
                        return;
                    }
                },
                None => handle_connection(&state, &cfg, stream),
            };
            if let Err(e) = result {
                eprintln!("http: {e}");
            }
        });
    }
    eprintln!("glasir: shutdown requested; draining active requests");
    let deadline = std::time::Instant::now() + IO_TIMEOUT;
    while live.load(Ordering::Relaxed) != 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Some(audit) = &cfg.audit
        && !audit.flush(IO_TIMEOUT)
    {
        eprintln!("glasir: audit flush did not complete before shutdown deadline");
    }
    Ok(())
}

/// Records an answered tool call, if this server keeps an audit log.
///
/// Only `tools/call` is recorded: `initialize`, `tools/list` and `ping` carry
/// no question and would bury the records that matter under handshake noise.
fn audit(
    cfg: &HttpConfig,
    who: Option<&str>,
    msg: &Value,
    reply: &Value,
    bytes: usize,
) -> Option<()> {
    let log = cfg.audit.as_ref()?;
    if msg["method"].as_str()? != "tools/call" {
        return None;
    }
    log.record(crate::audit::Record {
        who: who.unwrap_or("anonymous").to_string(),
        tool: msg["params"]["name"].as_str().unwrap_or("?").to_string(),
        args: msg["params"]["arguments"].to_string(),
        bytes,
        // A tool that rejects its arguments answers with `isError`, not a
        // protocol error — worth distinguishing in a log that is read to find
        // out what someone was trying to do.
        ok: !reply["result"]["isError"].as_bool().unwrap_or(false),
    });
    Some(())
}

/// Who is asking, or `Err(())` if they may not ask at all.
///
/// Per-user tokens take precedence: once the file holds any, the shared
/// `--token` is not a second way in — an operator who issues tokens has stated
/// that everyone should be identifiable, and leaving an anonymous door open
/// beside them would defeat both the attribution and the revocation.
///
/// `None` means "nobody is required to identify", which is the single-machine
/// loopback case. `serve` refuses to start in that state on a non-loopback
/// address, so this cannot silently be the answer on an open port.
pub fn authenticate(cfg: &HttpConfig, header: Option<&str>) -> Result<Option<String>, ()> {
    let given = header.and_then(|v| v.strip_prefix("Bearer "));
    if cfg.tokens.configured() {
        return cfg
            .tokens
            .identify(given, crate::auth::now())
            .map(Some)
            .ok_or(());
    }
    if let Some(expected) = &cfg.token {
        // Compared in constant time: a byte-by-byte early exit leaks the
        // shared secret's prefix to anyone who can time the responses, and this
        // one secret is the whole door.
        return match given {
            Some(g) if constant_time_eq(g.as_bytes(), expected.as_bytes()) => {
                Ok(Some("shared-token".into()))
            }
            _ => Err(()),
        };
    }
    Ok(None)
}

/// Equal, in time that does not depend on where they first differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether this configuration may serve on this address at all.
///
/// A graph of someone's source on every interface with no authentication is
/// the disclosure the transport spec warns about. It used to be a warning,
/// which is a line in a log nobody reads; now it refuses to start. Loopback is
/// unchanged — that is the single-machine case and must not get harder.
pub fn refuses_to_start(cfg: &HttpConfig) -> bool {
    (cfg.behind_control_plane
        && (!cfg.tokens.configured()
            || (!is_loopback(&cfg.addr) && cfg.control_plane_cidr.is_none())))
        || (!is_loopback(&cfg.addr) && cfg.token.is_none() && !cfg.tokens.configured())
}

fn is_loopback(addr: &str) -> bool {
    addr.starts_with("127.") || addr.starts_with("localhost:") || addr.starts_with("[::1]")
}

/// An `Origin` a browser could send. Only same-origin loopback is allowed: a
/// page on any other origin reaching a local server is the rebinding attack the
/// spec warns about, and a real MCP client sends no Origin at all.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let host = origin.split("://").nth(1).unwrap_or(origin);
    // Split the port off before comparing: a prefix test alone accepts
    // `localhost.evil.com`, which is exactly the attack being defended against.
    let host = host.split(['/', ':']).next().unwrap_or(host);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

/// Turns a `--http` value into a bind address. A bare port means loopback:
/// serving a graph of someone's source on every interface is a disclosure, and
/// a hostile web page cannot reach that loopback listener. The spec asks for
/// local binding unless stated otherwise.
pub fn addr_for(value: &str) -> String {
    if value.contains(':') {
        value.to_string()
    } else {
        format!("127.0.0.1:{value}")
    }
}

/// Where the metadata document lives, per RFC 9728.
const METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// Liveness, answered without a credential — see the handler.
const HEALTH_PATH: &str = "/health";
/// Readiness proves a graph was loaded and exposes its precise freshness to
/// an orchestrator. Unlike `/health`, it is a deployment decision, not merely
/// a liveness probe.
const READY_PATH: &str = "/ready";

/// What an operator's existing monitoring needs, in the format it already
/// scrapes.
///
/// `/health` answers *whether* the service is alive; this answers how it is
/// doing — which tool is being called, how often it fails, and how long an
/// answer takes. Without it Glasir is the one service in a rack that cannot be
/// watched like the others.
///
/// Hand-rolled rather than a Prometheus crate: the exposition format is a name,
/// a number and a newline, and the whole of what is worth exporting here is
/// four counters and a histogram. A dependency for that is a dependency to
/// keep patched.
///
/// Counters are per tool because that is the axis an operator acts on — a slow
/// `impact` and a slow `query_graph` have different causes — and the tool name
/// comes from a fixed list, never from the request, or a caller could mint
/// unbounded label values by asking for tools that do not exist.
#[derive(Default)]
pub struct Metrics {
    /// Answered tool calls, by tool.
    calls: [AtomicU64; TOOLS.len()],
    /// Of those, the ones that answered `isError`.
    errors: [AtomicU64; TOOLS.len()],
    /// Microseconds spent, by tool. Summed rather than averaged, so a scraper
    /// can divide by the count and get the mean over *its* interval.
    micros: [AtomicU64; TOOLS.len()],
    /// Requests refused before a tool was reached: 401, 403, 400.
    refused: AtomicU64,
    /// Cumulative buckets, in milliseconds. A histogram rather than a mean,
    /// because a mean hides the tail an operator is paged about.
    buckets: [AtomicU64; LATENCY_BOUNDS.len()],
}

/// Every tool this server answers. A fixed list, so the label set is bounded.
const TOOLS: [&str; 9] = [
    "query_graph",
    "overview",
    "shortest_path",
    "explain_node",
    "impact",
    "cycles",
    "get_code_snippet",
    "find_callers",
    "detect_changes",
];

/// Upper bounds in milliseconds. Chosen around what this server measures:
/// a warm `impact` is 0.1 ms and `overview` on a 1M-line tree is 28 ms, so the
/// interesting range is sub-millisecond to tens of milliseconds, with the last
/// bucket catching anything pathological.
const LATENCY_BOUNDS: [u64; 6] = [1, 5, 25, 100, 500, 2000];

impl Metrics {
    /// Records one answered tool call. Unknown tool names are dropped rather
    /// than counted under a catch-all: they cannot occur — `handle` refuses
    /// them before this is reached — and a bucket nobody can explain is worse
    /// than no bucket.
    pub fn record(&self, tool: &str, ok: bool, elapsed: std::time::Duration) {
        let Some(i) = TOOLS.iter().position(|t| *t == tool) else {
            return;
        };
        self.calls[i].fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.errors[i].fetch_add(1, Ordering::Relaxed);
        }
        self.micros[i].fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
        let ms = elapsed.as_millis() as u64;
        for (b, bound) in LATENCY_BOUNDS.iter().enumerate() {
            if ms <= *bound {
                self.buckets[b].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn refuse(&self) {
        self.refused.fetch_add(1, Ordering::Relaxed);
    }

    /// The Prometheus text exposition format.
    fn render(&self, nodes: usize, symbols: usize) -> String {
        let mut out = String::new();
        out.push_str("# HELP glasir_tool_calls_total Answered tool calls.\n");
        out.push_str("# TYPE glasir_tool_calls_total counter\n");
        for (i, tool) in TOOLS.iter().enumerate() {
            let n = self.calls[i].load(Ordering::Relaxed);
            out.push_str(&format!("glasir_tool_calls_total{{tool=\"{tool}\"}} {n}\n"));
        }
        out.push_str("# HELP glasir_tool_errors_total Tool calls answered with isError.\n");
        out.push_str("# TYPE glasir_tool_errors_total counter\n");
        for (i, tool) in TOOLS.iter().enumerate() {
            let n = self.errors[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "glasir_tool_errors_total{{tool=\"{tool}\"}} {n}\n"
            ));
        }
        out.push_str("# HELP glasir_tool_seconds_total Time spent answering, by tool.\n");
        out.push_str("# TYPE glasir_tool_seconds_total counter\n");
        for (i, tool) in TOOLS.iter().enumerate() {
            let us = self.micros[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "glasir_tool_seconds_total{{tool=\"{tool}\"}} {:.6}\n",
                us as f64 / 1e6
            ));
        }
        out.push_str("# HELP glasir_requests_refused_total Requests refused before a tool ran.\n");
        out.push_str("# TYPE glasir_requests_refused_total counter\n");
        out.push_str(&format!(
            "glasir_requests_refused_total {}\n",
            self.refused.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP glasir_tool_duration_ms Answer latency.\n");
        out.push_str("# TYPE glasir_tool_duration_ms histogram\n");
        let mut total = 0;
        for (b, bound) in LATENCY_BOUNDS.iter().enumerate() {
            let n = self.buckets[b].load(Ordering::Relaxed);
            total = total.max(n);
            out.push_str(&format!(
                "glasir_tool_duration_ms_bucket{{le=\"{bound}\"}} {n}\n"
            ));
        }
        let all: u64 = self.calls.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        let _ = total;
        out.push_str(&format!(
            "glasir_tool_duration_ms_bucket{{le=\"+Inf\"}} {all}\n"
        ));
        out.push_str(&format!("glasir_tool_duration_ms_count {all}\n"));
        let us: u64 = self.micros.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        out.push_str(&format!(
            "glasir_tool_duration_ms_sum {}\n",
            us as f64 / 1000.0
        ));
        // The graph itself, so one scrape answers both "is it working" and
        // "what is it serving".
        out.push_str("# HELP glasir_graph_nodes Nodes in the served graph.\n");
        out.push_str("# TYPE glasir_graph_nodes gauge\n");
        out.push_str(&format!("glasir_graph_nodes {nodes}\n"));
        out.push_str("# HELP glasir_graph_symbols Symbols in the registry.\n");
        out.push_str("# TYPE glasir_graph_symbols gauge\n");
        out.push_str(&format!("glasir_graph_symbols {symbols}\n"));
        out
    }
}

const METRICS_PATH: &str = "/metrics";

/// The name this instance is reached under, for the URL in a 401 challenge.
///
/// Two requirements pull apart. Behind the TLS proxy the README recommends,
/// the bound address is `127.0.0.1:8899` and useless as a name — but `Host`
/// is set by whoever calls, and echoing it hands out a redirect under our own
/// name. Measured: `evil.example.com` reached the challenge before this check
/// existed. "Loopback, so any `Host` is harmless" was the first version and is
/// wrong for the reason `origin_allowed` exists — a page in a browser on the
/// same machine sets whatever it likes.
///
/// So `Host` is honoured only when it names something we could be, and
/// `--public-url` is how an operator states what cannot be derived.
fn origin_of(cfg: &HttpConfig, host: Option<&str>) -> String {
    // Configured wins over anything a request claims: it is the one name that
    // was not supplied by the caller.
    if let Some(url) = &cfg.public_url {
        return url.trim_end_matches('/').to_string();
    }
    // https only where it can be true. A loopback server has no certificate,
    // and claiming https about it hands the client a URL it cannot reach.
    let scheme = if is_loopback(&cfg.addr) {
        "http"
    } else {
        "https"
    };
    let host = match host {
        Some(h) if h == cfg.addr => h,
        // A loopback server is reached under several equally valid names
        // (`localhost:7891`, `127.0.0.1:7891`), so the bind address alone is
        // too strict — but the name still has to be a loopback one.
        Some(h) if is_loopback(&cfg.addr) && origin_allowed(Some(h)) => h,
        _ => cfg.addr.as_str(),
    };
    format!("{scheme}://{host}")
}

/// This server's canonical URI, as RFC 8707 §2 defines a resource identifier:
/// the endpoint a token would be bound to. No fragment, no trailing slash.
pub fn resource_uri(cfg: &HttpConfig, host: Option<&str>) -> String {
    format!("{}/mcp", origin_of(cfg, host))
}

/// The Protected Resource Metadata document (RFC 9728 §2).
///
/// `resource` is the only required field and the only one stated truthfully
/// here. `authorization_servers` is deliberately absent: credentials come from
/// an operator running `glasir token add`, not from an OAuth issuer, and
/// naming one that does not exist sends a caller into a discovery that
/// dead-ends — worse than saying nothing.
fn resource_metadata(cfg: &HttpConfig, host: Option<&str>) -> String {
    json!({
        "resource": resource_uri(cfg, host),
        "bearer_methods_supported": ["header"],
        "resource_name": "glasir",
    })
    .to_string()
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn read_request<R: Read>(stream: &mut BufReader<R>) -> std::io::Result<Option<Request>> {
    let mut start = String::new();
    if stream.read_line(&mut start)? == 0 {
        return Ok(None); // client closed
    }
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    // Bounded for the same reason as the body: an endless header stream is a
    // second route to the same exhaustion, and no real request needs more.
    const MAX_HEADERS: usize = 100;
    loop {
        if headers.len() >= MAX_HEADERS {
            return Err(std::io::Error::other("too many headers"));
        }
        let mut line = String::new();
        if stream.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    // Refused before allocating: the header is attacker-controlled and arrives
    // before authentication.
    if length > MAX_BODY {
        return Err(std::io::Error::other(format!(
            "request body of {length} bytes exceeds the {MAX_BODY}-byte limit"
        )));
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        stream.read_exact(&mut body)?;
    }

    Ok(Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

fn respond<W: Write>(
    stream: &mut W,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    respond_with(stream, status, content_type, body, &[])
}

/// The same, with extra headers — `WWW-Authenticate` is the only user so far.
fn respond_with<W: Write>(
    stream: &mut W,
    status: &str,
    content_type: &str,
    body: &str,
    extra: &[(&str, &str)],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    for (k, v) in extra {
        write!(stream, "{k}: {v}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n{body}")?;
    stream.flush()
}

fn handle_connection<S: Read + Write>(
    state: &Published<ServedState>,
    cfg: &HttpConfig,
    mut stream: S,
) -> std::io::Result<()> {
    // Read the request through a borrow so `stream` stays usable for the
    // reply: TLS gives back one object that both reads and writes, where a
    // `TcpStream` could simply be borrowed twice.
    let Some(req) = read_request(&mut BufReader::new(&mut stream))? else {
        return Ok(());
    };

    if !origin_allowed(req.header("origin")) {
        cfg.metrics.refuse();
        return respond(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            "origin not allowed",
        );
    }
    // Unauthenticated for the same reason as the metadata below: a prober has
    // no credential to offer. It discloses only what `overview` already does.
    if req.path == HEALTH_PATH {
        let state = state.load();
        return respond(
            &mut stream,
            "200 OK",
            "application/json",
            &format!(
                "{{\"status\":\"ok\",\"nodes\":{},\"symbols\":{}}}",
                state.snap.width(),
                state.registry.len()
            ),
        );
    }
    if req.path == READY_PATH {
        let state = state.load();
        let age = crate::auth::now().saturating_sub(state.now);
        let stale = cfg.max_index_age.is_some_and(|limit| age > limit.as_secs());
        let generation = cfg
            .reindexed
            .as_ref()
            .map(|n| n.load(Ordering::Relaxed))
            .unwrap_or(0);
        let error = cfg
            .reindex_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let error = error
            .map(|value| {
                serde_json::to_string(&value).unwrap_or_else(|_| "\"unserializable error\"".into())
            })
            .unwrap_or_else(|| "null".into());
        return respond(
            &mut stream,
            if stale {
                "503 Service Unavailable"
            } else {
                "200 OK"
            },
            "application/json",
            &format!(
                "{{\"status\":\"{}\",\"indexed_at\":{},\"age_seconds\":{},\"generation\":{},\"nodes\":{},\"symbols\":{},\"last_reindex_error\":{error}}}",
                if stale { "stale" } else { "ready" },
                state.now,
                age,
                generation,
                state.snap.width(),
                state.registry.len(),
            ),
        );
    }
    // Unauthenticated for the same reason `/health` is: a scraper is a
    // machine on the operator's own network with no credential to offer, and
    // what it reads — call counts and latencies — discloses less than
    // `overview` already does to any caller.
    if req.path == METRICS_PATH {
        let state = state.load();
        return respond(
            &mut stream,
            "200 OK",
            "text/plain; version=0.0.4",
            &cfg.metrics.render(state.snap.width(), state.registry.len()),
        );
    }
    // Answered before authentication, which is the whole point: it is fetched
    // *because* the caller was refused, so gating it would close the loop it
    // exists to open. Nothing here is secret — the address they already
    // reached, and where to put the credential.
    if req.path == METADATA_PATH {
        return respond(
            &mut stream,
            "200 OK",
            "application/json",
            &resource_metadata(cfg, req.header("host")),
        );
    }

    let identity = match authenticate(cfg, req.header("authorization")) {
        Ok(who) => who,
        Err(()) => {
            // RFC 9728 §5.1. A bare 401 says only "no", leaving the caller
            // nothing to act on. Authorization is OPTIONAL in MCP and the
            // scheme here is a shared or per-user credential, so this is not a
            // violation being fixed — but the discovery half is one header and
            // one document.
            let challenge = format!(
                "Bearer resource_metadata=\"{}{METADATA_PATH}\"",
                origin_of(cfg, req.header("host"))
            );
            cfg.metrics.refuse();
            return respond_with(
                &mut stream,
                "401 Unauthorized",
                "text/plain",
                "bad, expired or missing token",
                &[("WWW-Authenticate", challenge.as_str())],
            );
        }
    };

    if req.path != "/mcp" {
        return respond(
            &mut stream,
            "404 Not Found",
            "text/plain",
            "the MCP endpoint is /mcp",
        );
    }

    match req.method.as_str() {
        "POST" => {
            let Ok(msg) = serde_json::from_str::<Value>(&req.body) else {
                cfg.metrics.refuse();
                let err = json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": "parse error"}});
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "application/json",
                    &err.to_string(),
                );
            };
            // Loaded per request, not per connection: a re-index that lands
            // between two requests on the same socket is picked up by the
            // second one.
            let state = state.load();
            // Timed around `handle` alone, not around the socket: what an
            // operator can act on is how long an answer takes to compute, and
            // a slow client would otherwise be reported as a slow tool.
            let started = std::time::Instant::now();
            match mcp::handle(&state.as_served(), &msg) {
                Some(reply) => {
                    let body = reply.to_string();
                    if msg["method"].as_str() == Some("tools/call") {
                        cfg.metrics.record(
                            msg["params"]["name"].as_str().unwrap_or("?"),
                            !reply["result"]["isError"].as_bool().unwrap_or(false),
                            started.elapsed(),
                        );
                    }
                    audit(cfg, identity.as_deref(), &msg, &reply, body.len());
                    respond(&mut stream, "200 OK", "application/json", &body)
                }
                // A notification is accepted with no body, per the spec.
                None => respond(&mut stream, "202 Accepted", "text/plain", ""),
            }
        }
        // A GET opens a stream the server pushes re-index notifications on.
        //
        // Under `--watch` the served graph *is* replaced while clients are
        // connected (`state.store` in `run_serve`), so a client that cached an
        // answer is holding a description of a tree that has moved on, with
        // nothing in the protocol telling it so. That is what this stream is
        // for. Without `--watch` nothing republishes and `reindexed` is `None`:
        // the stream then stays open and silent, which is honest.
        "GET" => stream_events(stream, cfg),
        // Sessions are not issued, so there is nothing for a client to end.
        "DELETE" => respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            "stateless server",
        ),
        _ => respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            "POST or GET /mcp",
        ),
    }
}

/// How often an open SSE stream checks whether the graph was republished.
///
/// A re-index is a human-scale event (a file save, then a debounce, then a
/// re-analysis), so sub-second polling would buy nothing a person could
/// perceive while waking every attached thread. Half a second is below the
/// threshold where a stale cached answer is worth noticing.
const EVENT_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// How long a silent stream waits before writing an SSE comment.
///
/// A stream that never writes is indistinguishable from a hung one to every
/// proxy between here and the client, and both will eventually close it. A
/// colon line is a comment in the SSE grammar: it costs three bytes, resets
/// those timers and is ignored by every conforming client.
const EVENT_KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(15);

/// Pushes `notifications/resources/updated` whenever the served graph is
/// replaced, until the client goes away.
///
/// The stream holds one of `MAX_CONNECTIONS` slots for as long as it is open,
/// which is why the write deadline matters: `serve` set a ten-second write
/// timeout, so a client that stops reading cannot wedge a slot forever — the
/// write fails and this returns. The read side is never touched, so the
/// `IO_TIMEOUT` on reads is irrelevant to a connection that only listens.
fn stream_events<W: Write>(mut stream: W, cfg: &HttpConfig) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n: connected\n\n"
    )?;
    stream.flush()?;

    // Nothing republishes without `--watch`, so there is no event this stream
    // could ever carry. Returning ends the connection and frees its slot,
    // rather than parking a thread on a counter that cannot move.
    let Some(counter) = cfg.reindexed.clone() else {
        return Ok(());
    };

    // The generation this client is caught up with. Starting from the current
    // value rather than zero is what keeps a client connecting to a
    // long-running server from being told to invalidate a cache it does not
    // have yet.
    let mut seen = counter.load(Ordering::Relaxed);
    let mut quiet = std::time::Duration::ZERO;
    loop {
        std::thread::sleep(EVENT_POLL);
        let now = counter.load(Ordering::Relaxed);
        if now != seen {
            seen = now;
            quiet = std::time::Duration::ZERO;
            // A notification carries no id: it is not a request and the client
            // must not answer it. `resources/updated` is the standard way to
            // say "what you have is stale"; the tools are unchanged, only the
            // graph behind them.
            let event = json!({
                "jsonrpc": "2.0",
                "method": "notifications/resources/updated",
                "params": {"uri": "glasir://graph"}
            });
            write!(stream, "data: {event}\n\n")?;
            stream.flush()?;
        } else {
            quiet += EVENT_POLL;
            if quiet >= EVENT_KEEPALIVE {
                quiet = std::time::Duration::ZERO;
                write!(stream, ": keep-alive\n\n")?;
                stream.flush()?;
            }
        }
    }
}

/// Serves the map: the page itself, and the viewport queries it streams from.
///
/// `answer` is handed a path and its query string and returns JSON; anything it
/// does not recognise falls through to the page.
pub fn serve_view(
    addr: &str,
    page: &str,
    answer: impl Fn(&str, &str) -> Option<String>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Ok(Some(req)) = read_request(&mut BufReader::new(&stream)) else {
            continue;
        };
        // Same rebinding defence as the MCP endpoint: this serves a map of
        // someone's source, so a foreign page must not be able to read it.
        if !origin_allowed(req.header("origin")) {
            let _ = respond(
                &mut stream,
                "403 Forbidden",
                "text/plain",
                "origin not allowed",
            );
            continue;
        }
        let (path, query) = req.path.split_once('?').unwrap_or((req.path.as_str(), ""));
        let _ = match answer(path, query) {
            Some(json) => respond(&mut stream, "200 OK", "application/json", &json),
            None => respond(&mut stream, "200 OK", "text/html; charset=utf-8", page),
        };
    }
    Ok(())
}

/// Reads one `f32` from a `key=value&…` query string.
pub fn param(query: &str, key: &str) -> Option<f32> {
    query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.parse().ok())
}

/// Reads one string value from a `key=value&…` query string.
pub fn text_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::ControlPlaneCidr;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn control_plane_cidr_is_canonical_and_never_matches_outside_peers() {
        let cidr = ControlPlaneCidr::parse("10.1.0.0/16").unwrap();
        assert!(cidr.contains(IpAddr::V4(Ipv4Addr::new(10, 1, 13, 7))));
        assert!(!cidr.contains(IpAddr::V4(Ipv4Addr::new(10, 2, 0, 1))));
        assert!(!cidr.contains(IpAddr::V6("::1".parse().unwrap())));
        assert!(ControlPlaneCidr::parse("10.1.3.0/16").is_err());
        assert!(ControlPlaneCidr::parse("10.1.0.0/33").is_err());
    }
}
