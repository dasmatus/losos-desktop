//! The DNS forwarder resolved sends every query to.
//!
//! resolved stays the system's resolver, with its cache, its stub on
//! 127.0.0.53, mDNS and LLMNR; the OS makes this forwarder its only global
//! server and takes the default route away from each link's servers
//! (nixos/modules/adblock.nix), so every query for the wider internet comes
//! here. A blocked name is answered on the spot; anything else goes,
//! unchanged, to the servers the network handed out, which resolved lists
//! in /run/systemd/resolve/resolv.conf for programs like this one.
//!
//! Queries go through a bounded queue to a few worker threads: resolved
//! caches and coalesces, so what reaches here is a desktop's worth of
//! cache misses, and a worker blocked on a slow upstream holds up only its
//! own query. When every worker is busy and the queue is full, a query is
//! dropped and the client retries, as DNS clients do. Every thread is
//! scoped to `run`, which shares its state by reference.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6, TcpListener, TcpStream, UdpSocket};
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::sync::RwLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::TrySendError;
use tracing::{debug, info, warn};

use crate::dns;
use crate::rules::{Blocklist, Verdict};

/// Where resolved lists every server it knows, links' included.
const RESOLV_CONF: &str = "/run/systemd/resolve/resolv.conf";
/// How long one upstream gets before the next is tried.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the compiled list and resolv.conf are checked for changes.
const RELOAD_EVERY: Duration = Duration::from_secs(15);
/// Worker threads answering UDP queries, and how many queries may wait
/// for one: enough for a burst of cache misses, few enough that a dead
/// upstream cannot pile up minutes of work.
const WORKERS: usize = 8;
const QUEUE: usize = 256;

/// The listening sockets: from systemd's socket activation, or bound here.
pub struct Sockets {
    pub udp: Vec<UdpSocket>,
    pub tcp: Vec<TcpListener>,
}

impl Sockets {
    /// Sockets systemd passed (`LISTEN_FDS`), sorted by type.
    pub fn from_systemd() -> Option<Self> {
        let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
        if pid != std::process::id() {
            return None;
        }
        let count: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
        let mut sockets = Self {
            udp: Vec::new(),
            tcp: Vec::new(),
        };
        for fd in 3..3 + count {
            let mut kind: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            // SAFETY: fd is one systemd passed us; SO_TYPE writes one int.
            let ok = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TYPE,
                    (&mut kind as *mut libc::c_int).cast(),
                    &mut len,
                )
            } == 0;
            if !ok {
                continue;
            }
            // SAFETY: systemd hands over ownership of each listed fd.
            match kind {
                libc::SOCK_DGRAM => sockets.udp.push(unsafe { UdpSocket::from_raw_fd(fd) }),
                libc::SOCK_STREAM => sockets.tcp.push(unsafe { TcpListener::from_raw_fd(fd) }),
                _ => {}
            }
        }
        Some(sockets)
    }

    /// Binds UDP and TCP on `addr`, for running outside systemd.
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        Ok(Self {
            udp: vec![UdpSocket::bind(addr)?],
            tcp: vec![TcpListener::bind(addr)?],
        })
    }

    fn local_addrs(&self) -> Vec<SocketAddr> {
        self.udp
            .iter()
            .filter_map(|s| s.local_addr().ok())
            .chain(self.tcp.iter().filter_map(|s| s.local_addr().ok()))
            .collect()
    }
}

/// What every query is checked and forwarded against, reloaded in place.
struct State {
    blocklist: Blocklist,
    upstreams: Vec<SocketAddr>,
}

struct Shared {
    state: RwLock<State>,
    list: PathBuf,
    /// Our own addresses, never forwarded to: resolved lists us among its
    /// servers.
    own: Vec<SocketAddr>,
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn load_list(path: &std::path::Path) -> Blocklist {
    match fs::read_to_string(path) {
        Ok(text) => {
            let list = Blocklist::read(&text);
            info!(domains = list.len(), "loaded the blocklist");
            list
        }
        Err(error) => {
            // Before the first update there is nothing to block yet;
            // forwarding still works.
            warn!(path = %path.display(), %error, "no blocklist yet; forwarding everything");
            Blocklist::default()
        }
    }
}

/// The servers resolved knows, minus resolved's own stubs and us.
pub fn upstreams(text: &str, own: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(server) = line.trim().strip_prefix("nameserver") else {
            continue;
        };
        let server = server.trim();
        let addr = match server.split_once('%') {
            // A link-local IPv6 server is only reachable through its link.
            Some((ip, link)) => {
                let Ok(ip) = ip.parse::<Ipv6Addr>() else {
                    continue;
                };
                let Ok(name) = std::ffi::CString::new(link) else {
                    continue;
                };
                // SAFETY: a NUL-terminated interface name.
                let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
                SocketAddr::V6(SocketAddrV6::new(ip, 53, 0, index))
            }
            None => match server.parse::<IpAddr>() {
                Ok(ip) => SocketAddr::new(ip, 53),
                Err(_) => continue,
            },
        };
        let stub = matches!(addr.ip(), IpAddr::V4(v4) if v4.octets()[..3] == [127, 0, 0] && matches!(v4.octets()[3], 53 | 54));
        if stub || own.iter().any(|o| o.ip() == addr.ip()) || out.contains(&addr) {
            continue;
        }
        out.push(addr);
    }
    out
}

fn load_upstreams(own: &[SocketAddr]) -> Vec<SocketAddr> {
    // Outside a resolved system, /etc/resolv.conf names the real servers;
    // under resolved it names only the stub, which is skipped.
    let text = fs::read_to_string(RESOLV_CONF)
        .or_else(|_| fs::read_to_string("/etc/resolv.conf"))
        .unwrap_or_default();
    let found = upstreams(&text, own);
    debug!(?found, "upstream servers");
    found
}

/// Answers `query`: blocked here, forwarded, or a SERVFAIL.
fn answer(shared: &Shared, query: &[u8], tcp: bool) -> Option<Vec<u8>> {
    let state = shared.state.read().ok()?;
    if let Some(question) = dns::question(query) {
        if let Verdict::Blocked(rule) = state.blocklist.verdict(&question.name) {
            debug!(name = %question.name, %rule, "blocked");
            return Some(dns::blocked(query, &question));
        }
    }
    let upstreams = state.upstreams.clone();
    drop(state);
    for upstream in upstreams {
        let result = if tcp {
            forward_tcp(upstream, query)
        } else {
            forward_udp(upstream, query)
        };
        match result {
            Ok(reply) => return Some(reply),
            Err(error) => debug!(%upstream, %error, "upstream failed"),
        }
    }
    dns::servfail(query)
}

fn forward_udp(upstream: SocketAddr, query: &[u8]) -> io::Result<Vec<u8>> {
    let bind: SocketAddr = if upstream.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let socket = UdpSocket::bind(bind)?;
    socket.connect(upstream)?;
    socket.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    socket.send(query)?;
    let mut buf = vec![0u8; 65535];
    loop {
        let n = socket.recv(&mut buf)?;
        // Only the reply to this query: same id. A connected socket already
        // drops packets from anyone else.
        if n >= 2 && buf[..2] == query[..2] {
            buf.truncate(n);
            return Ok(buf);
        }
    }
}

fn forward_tcp(upstream: SocketAddr, query: &[u8]) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&upstream, UPSTREAM_TIMEOUT)?;
    stream.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    stream.set_write_timeout(Some(UPSTREAM_TIMEOUT))?;
    stream.write_all(&(query.len() as u16).to_be_bytes())?;
    stream.write_all(query)?;
    read_tcp_message(&mut stream)
}

fn read_tcp_message(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let mut message = vec![0u8; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut message)?;
    Ok(message)
}

fn serve_tcp_client(shared: &Shared, mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    // A client may send several queries on one connection.
    loop {
        let query = match read_tcp_message(&mut stream) {
            Ok(q) => q,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if let Some(reply) = answer(shared, &query, true) {
            stream.write_all(&(reply.len() as u16).to_be_bytes())?;
            stream.write_all(&reply)?;
        }
    }
}

/// Serves until the process is stopped.
pub fn run(sockets: Sockets, list: PathBuf) -> io::Result<()> {
    let own = sockets.local_addrs();
    let shared = Shared {
        state: RwLock::new(State {
            blocklist: load_list(&list),
            upstreams: load_upstreams(&own),
        }),
        list,
        own,
    };
    let shared = &shared;

    // One UDP query per message: the datagram and who sent it, with the
    // socket it came on to answer from.
    let (queue, queries) = crossbeam_channel::bounded::<(Vec<u8>, SocketAddr, &UdpSocket)>(QUEUE);

    thread::scope(|s| {
        // Reloads: the blocklist after an update, the upstreams after the
        // network changes. Checked by modification time, so an update needs
        // no signal to reach a running forwarder.
        s.spawn(move || {
            let mut list_time = modified(&shared.list);
            let mut conf_time = modified(std::path::Path::new(RESOLV_CONF));
            loop {
                thread::sleep(RELOAD_EVERY);
                let now = modified(&shared.list);
                if now != list_time {
                    list_time = now;
                    let list = load_list(&shared.list);
                    if let Ok(mut state) = shared.state.write() {
                        state.blocklist = list;
                    }
                }
                let now = modified(std::path::Path::new(RESOLV_CONF));
                if now != conf_time {
                    conf_time = now;
                    let upstreams = load_upstreams(&shared.own);
                    if let Ok(mut state) = shared.state.write() {
                        state.upstreams = upstreams;
                    }
                }
            }
        });

        for _ in 0..WORKERS {
            let queries = queries.clone();
            s.spawn(move || {
                for (query, from, socket) in queries {
                    let started = Instant::now();
                    if let Some(reply) = answer(shared, &query, false) {
                        let _ = socket.send_to(&reply, from);
                    }
                    debug!(elapsed = ?started.elapsed(), "answered");
                }
            });
        }

        // A TCP client gets a thread of its own for as long as it stays
        // connected; there are few, and each may send several queries.
        for listener in &sockets.tcp {
            s.spawn(move || {
                for stream in listener.incoming().flatten() {
                    s.spawn(move || {
                        if let Err(error) = serve_tcp_client(shared, stream) {
                            debug!(%error, "TCP client");
                        }
                    });
                }
            });
        }

        for socket in &sockets.udp {
            let queue = queue.clone();
            s.spawn(move || {
                let mut buf = vec![0u8; 65535];
                loop {
                    let Ok((n, from)) = socket.recv_from(&mut buf) else {
                        continue;
                    };
                    match queue.try_send((buf[..n].to_vec(), from, socket)) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            debug!(%from, "query dropped, every worker is busy")
                        }
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_resolveds_servers() {
        let own: Vec<SocketAddr> = vec!["127.0.0.153:53".parse().unwrap()];
        let text = "# generated\nnameserver 127.0.0.153\nnameserver 192.168.1.1\nnameserver 127.0.0.53\nnameserver 2001:db8::1\nnameserver 192.168.1.1\nsearch lan\n";
        let found = upstreams(text, &own);
        assert_eq!(
            found,
            vec![
                "192.168.1.1:53".parse::<SocketAddr>().unwrap(),
                "[2001:db8::1]:53".parse::<SocketAddr>().unwrap()
            ]
        );
    }
}
