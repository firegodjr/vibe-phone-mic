//! LAN discovery: UDP broadcast, localhost (adb forward) probe and a /24 TCP probe sweep.

use crate::protocol::{self, DiscoveryReply, Hello, Mode, DEFAULT_PORT, DISCOVER_MSG};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A phone found on the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub name: String,
    /// How it was found: "udp", "localhost" or "sweep".
    pub source: String,
}

impl Server {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Scan configuration. Everything is injectable so tests can run on loopback.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Total scan budget.
    pub timeout: Duration,
    /// UDP discovery targets (broadcast addresses).
    pub udp_targets: Vec<SocketAddr>,
    /// Addresses probed directly over TCP (e.g. 127.0.0.1 for adb forward).
    pub probe_targets: Vec<SocketAddr>,
    /// /24 networks to sweep (any address inside the /24).
    pub sweep_networks: Vec<Ipv4Addr>,
    /// Port used for the sweep.
    pub sweep_port: u16,
    /// Connect timeout for each TCP probe.
    pub probe_timeout: Duration,
    /// Max concurrent sweep probes.
    pub concurrency: usize,
}

impl ScanConfig {
    /// Default config built from the local interfaces.
    pub fn system(timeout: Duration) -> ScanConfig {
        let mut udp_targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, DEFAULT_PORT))];
        let mut sweep_networks = Vec::new();
        for iface in local_ipv4_interfaces() {
            if let Some(b) = iface.broadcast {
                let t = SocketAddr::from((b, DEFAULT_PORT));
                if !udp_targets.contains(&t) {
                    udp_targets.push(t);
                }
            }
            // Only sweep real LANs (prefix /16../30). Skip /32 tunnels (e.g. tailscale).
            if (16..=30).contains(&iface.prefix) && iface.ip.is_private() {
                let net = mask24(iface.ip);
                if !sweep_networks.contains(&net) {
                    sweep_networks.push(net);
                }
            }
        }
        ScanConfig {
            timeout,
            udp_targets,
            probe_targets: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))],
            sweep_networks,
            sweep_port: DEFAULT_PORT,
            probe_timeout: Duration::from_millis(300),
            concurrency: 64,
        }
    }

    /// Config with nothing enabled; tests fill in what they want.
    pub fn empty(timeout: Duration) -> ScanConfig {
        ScanConfig {
            timeout,
            udp_targets: vec![],
            probe_targets: vec![],
            sweep_networks: vec![],
            sweep_port: DEFAULT_PORT,
            probe_timeout: Duration::from_millis(300),
            concurrency: 64,
        }
    }
}

fn mask24(ip: Ipv4Addr) -> Ipv4Addr {
    let o = ip.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 0)
}

#[derive(Debug, Clone)]
pub struct Iface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub prefix: u8,
    pub broadcast: Option<Ipv4Addr>,
}

/// Non-loopback IPv4 interfaces.
pub fn local_ipv4_interfaces() -> Vec<Iface> {
    let mut out = Vec::new();
    if let Ok(list) = if_addrs::get_if_addrs() {
        for i in list {
            if i.is_loopback() {
                continue;
            }
            if let if_addrs::IfAddr::V4(v4) = &i.addr {
                let prefix = u32::from(v4.netmask).count_ones() as u8;
                out.push(Iface { name: i.name.clone(), ip: v4.ip, prefix, broadcast: v4.broadcast });
            }
        }
    }
    out
}

/// Connect in probe mode and read HELLO.
pub fn probe(addr: SocketAddr, timeout: Duration) -> io::Result<Hello> {
    let mut s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout.max(Duration::from_millis(500))))?;
    s.set_write_timeout(Some(timeout))?;
    s.set_nodelay(true)?;
    s.write_all(&protocol::encode_handshake(Mode::Probe))?;
    match protocol::read_first_frame(&mut s)? {
        Some(h) => Ok(h),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed before HELLO")),
    }
}

/// UDP broadcast discovery.
pub fn udp_discover(targets: &[SocketAddr], timeout: Duration) -> Vec<Server> {
    let mut found = Vec::new();
    if targets.is_empty() {
        return found;
    }
    let sock = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(_) => return found,
    };
    let _ = sock.set_broadcast(true);
    let send_all = |sock: &UdpSocket| {
        for t in targets {
            let _ = sock.send_to(DISCOVER_MSG, t);
        }
    };
    send_all(&sock);
    let start = Instant::now();
    let mut resent = false;
    let mut buf = [0u8; 2048];
    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            break;
        }
        // Re-send once halfway in case the first datagram was lost.
        if !resent && elapsed >= timeout / 2 {
            send_all(&sock);
            resent = true;
        }
        let wait = (timeout - elapsed).min(Duration::from_millis(100)).max(Duration::from_millis(1));
        let _ = sock.set_read_timeout(Some(wait));
        if let Ok((n, from)) = sock.recv_from(&mut buf)
            && let Some(r) = DiscoveryReply::parse(&buf[..n]) {
                found.push(Server { host: from.ip().to_string(), port: r.port, name: r.name, source: "udp".into() });
            }
    }
    found
}

/// Probe a list of addresses with bounded concurrency.
pub fn probe_many(addrs: Vec<SocketAddr>, timeout: Duration, concurrency: usize, deadline: Instant, source: &str) -> Vec<Server> {
    let addrs = Arc::new(addrs);
    let next = Arc::new(AtomicUsize::new(0));
    let found = Arc::new(Mutex::new(Vec::new()));
    let workers = concurrency.clamp(1, addrs.len().max(1));
    let mut handles = Vec::new();
    for _ in 0..workers {
        let (addrs, next, found) = (addrs.clone(), next.clone(), found.clone());
        let source = source.to_string();
        handles.push(thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= addrs.len() || Instant::now() >= deadline {
                break;
            }
            let addr = addrs[i];
            let remaining = deadline.saturating_duration_since(Instant::now());
            let t = timeout.min(remaining).max(Duration::from_millis(20));
            if let Ok(h) = probe(addr, t) {
                found.lock().unwrap().push(Server {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                    name: h.name,
                    source: source.clone(),
                });
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Arc::try_unwrap(found).map(|m| m.into_inner().unwrap()).unwrap_or_default()
}

/// Run all discovery methods in parallel and return a deduplicated list.
pub fn scan(cfg: &ScanConfig) -> Vec<Server> {
    let deadline = Instant::now() + cfg.timeout;
    let udp = {
        let targets = cfg.udp_targets.clone();
        let t = cfg.timeout;
        thread::spawn(move || udp_discover(&targets, t))
    };
    let local = {
        let targets = cfg.probe_targets.clone();
        let (t, c) = (cfg.probe_timeout, cfg.concurrency);
        thread::spawn(move || probe_many(targets, t, c, deadline, "localhost"))
    };
    let sweep = {
        let mut addrs = Vec::new();
        for net in &cfg.sweep_networks {
            let o = net.octets();
            for last in 1..=254u8 {
                addrs.push(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(o[0], o[1], o[2], last), cfg.sweep_port)));
            }
        }
        let (t, c) = (cfg.probe_timeout, cfg.concurrency);
        thread::spawn(move || probe_many(addrs, t, c, deadline, "sweep"))
    };
    let mut all = Vec::new();
    // Order matters for dedupe: UDP names first, then localhost, then sweep.
    all.extend(udp.join().unwrap_or_default());
    all.extend(local.join().unwrap_or_default());
    all.extend(sweep.join().unwrap_or_default());
    dedupe(all)
}

/// Deduplicate by host:port, keeping the first entry but filling an empty name.
pub fn dedupe(list: Vec<Server>) -> Vec<Server> {
    let mut map: BTreeMap<(String, u16), Server> = BTreeMap::new();
    let mut order = Vec::new();
    for s in list {
        let key = (s.host.clone(), s.port);
        match map.get_mut(&key) {
            Some(existing) => {
                if existing.name.is_empty() {
                    existing.name = s.name;
                }
            }
            None => {
                order.push(key.clone());
                map.insert(key, s);
            }
        }
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srv(host: &str, port: u16, name: &str, source: &str) -> Server {
        Server { host: host.into(), port, name: name.into(), source: source.into() }
    }

    #[test]
    fn dedupe_keeps_first_and_fills_name() {
        let out = dedupe(vec![
            srv("10.0.0.2", 48777, "", "udp"),
            srv("10.0.0.3", 48777, "B", "sweep"),
            srv("10.0.0.2", 48777, "A", "sweep"),
            srv("10.0.0.2", 1, "C", "sweep"),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], srv("10.0.0.2", 48777, "A", "udp"));
        assert_eq!(out[1].host, "10.0.0.3");
    }

    #[test]
    fn system_config_sane() {
        let c = ScanConfig::system(Duration::from_secs(1));
        assert!(c.udp_targets.contains(&SocketAddr::from((Ipv4Addr::BROADCAST, DEFAULT_PORT))));
        assert!(c.probe_targets.contains(&SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))));
        assert!(c.sweep_networks.iter().all(|n| n.octets()[3] == 0));
    }
}
