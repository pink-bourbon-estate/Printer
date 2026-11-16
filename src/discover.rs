//! Finding the printer on the local network: mDNS first, then a direct
//! subnet sweep, then remembering whatever was found so it is never "lost"
//! again.

use crate::{escl::EsclScanner, ippclient::IppPrinter};
use std::collections::HashMap;
use std::net::{IpAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub host: String,
    pub name: String,
    pub model: String,
    pub ipp_port: u16,
    pub ipp_resource: String,
    pub scan_port: Option<u16>,
    pub scan_resource: String,
    pub via: String,
}

impl Device {
    pub fn new(host: &str) -> Self {
        Device { host: host.to_string(), ipp_port: 631, ipp_resource: "/ipp/print".into(), scan_resource: "/eSCL".into(), ..Default::default() }
    }

    pub fn label(&self) -> String {
        let bits: Vec<&str> = [self.model.as_str(), self.name.as_str(), self.host.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
        bits.join(" - ")
    }

    pub fn printer(&self) -> IppPrinter {
        IppPrinter::new(&self.host, self.ipp_port, &self.ipp_resource)
    }

    pub fn scanner(&self) -> Option<EsclScanner> {
        self.scan_port.map(|p| EsclScanner::new(&self.host, p, &self.scan_resource))
    }

    fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.host,
            self.name,
            self.model,
            self.ipp_port,
            self.ipp_resource,
            self.scan_port.map(|p| format!("{p}:{}", self.scan_resource)).unwrap_or_default()
        )
    }

    fn from_line(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.splitn(6, '\t').collect();
        if parts.len() < 5 {
            return None;
        }
        let (scan_port, scan_resource) = parts
            .get(5)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.split_once(':'))
            .map(|(p, r)| (p.parse().ok(), r.to_string()))
            .unwrap_or((None, "/eSCL".into()));
        Some(Device {
            host: parts[0].to_string(),
            name: parts[1].to_string(),
            model: parts[2].to_string(),
            ipp_port: parts[3].parse().unwrap_or(631),
            ipp_resource: parts[4].to_string(),
            scan_port,
            scan_resource,
            via: "saved".into(),
        })
    }
}

fn config_path() -> std::path::PathBuf {
    let base = dirs_config_home().join("printer-rs");
    let _ = std::fs::create_dir_all(&base);
    base.join("printer.tsv")
}

fn dirs_config_home() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return xdg.into();
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join(".config");
    }
    std::env::temp_dir()
}

pub fn save_device(dev: &Device) -> std::io::Result<std::path::PathBuf> {
    let path = config_path();
    std::fs::write(&path, dev.to_line())?;
    Ok(path)
}

pub fn load_device() -> Option<Device> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    Device::from_line(text.trim())
}

/// Checks one host for IPP and eSCL, filling in the ports it actually
/// answers on rather than guessing.
pub fn inspect_host(host: &str, connect_timeout: Duration) -> Option<Device> {
    let has_ipp = port_open(host, 631, connect_timeout);
    let has_http = port_open(host, 80, connect_timeout) || port_open(host, 8080, connect_timeout);
    if !has_ipp && !has_http {
        return None;
    }

    let mut dev = Device::new(host);
    dev.via = "probed".into();
    let mut ok = false;

    if has_ipp
        && let Some(printer) = crate::ippclient::probe(host, 631) {
            dev.ipp_port = printer.port;
            dev.ipp_resource = printer.resource.clone();
            if let Ok(a) = printer.get_printer_attributes() {
                dev.name = a.get("printer-name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                dev.model = a.get("printer-make-and-model").and_then(|v| v.as_str()).unwrap_or("").to_string();
            }
            ok = true;
        }
    if let Some(scanner) = crate::escl::probe(host) {
        dev.scan_port = Some(scanner.port);
        dev.scan_resource = scanner.root.clone();
        if dev.model.is_empty()
            && let Ok(caps) = scanner.capabilities() {
                dev.model = caps.make_and_model;
            }
        ok = true;
    }

    ok.then_some(dev)
}

fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    let addr = format!("{host}:{port}");
    addr.parse()
        .ok()
        .map(|a: std::net::SocketAddr| TcpStream::connect_timeout(&a, timeout).is_ok())
        .unwrap_or(false)
}

/// Works out the local /24 this machine is likely on, by opening a UDP
/// "connection" to a reserved address (sends nothing) and reading back the
/// local address the OS picked.
pub fn local_subnet_prefix() -> Option<String> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:1").ok()?;
    let addr = sock.local_addr().ok()?;
    if let IpAddr::V4(ip) = addr.ip() {
        let o = ip.octets();
        return Some(format!("{}.{}.{}", o[0], o[1], o[2]));
    }
    None
}

/// Sweeps a /24 for anything answering IPP or eSCL, in parallel.
pub fn discover_subnet(log: impl Fn(&str) + Sync) -> Vec<Device> {
    let Some(prefix) = local_subnet_prefix() else {
        log("Could not work out the local network to sweep.");
        return vec![];
    };
    log(&format!("Sweeping {prefix}.0/24..."));

    let hosts: Vec<String> = (1..255).map(|i| format!("{prefix}.{i}")).collect();
    let results: Vec<Option<Device>> = std::thread::scope(|scope| {
        let chunks: Vec<_> = hosts
            .chunks(32)
            .map(|chunk| scope.spawn(move || chunk.iter().map(|h| inspect_host(h, Duration::from_millis(250))).collect::<Vec<_>>()))
            .collect();
        chunks.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });

    let found: Vec<Device> = results.into_iter().flatten().collect();
    for d in &found {
        log(&format!("Found {}", d.label()));
    }
    found
}

/// Browses mDNS for `_ipp._tcp` and `_uscan._tcp` for a few seconds.
pub fn discover_mdns(timeout: Duration, log: impl Fn(&str)) -> Vec<Device> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let Ok(mdns) = ServiceDaemon::new() else {
        log("Could not start mDNS.");
        return vec![];
    };

    let mut devices: HashMap<String, Device> = HashMap::new();
    let services = ["_ipp._tcp.local.", "_uscan._tcp.local."];
    let mut receivers = Vec::new();
    for svc in services {
        if let Ok(r) = mdns.browse(svc) {
            receivers.push((svc, r));
        }
    }

    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        for (svc, r) in &receivers {
            while let Ok(event) = r.try_recv() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let Some(addr) = info.get_addresses().iter().next() else { continue };
                    let host = addr.to_string();
                    let props = info.get_properties();
                    let dev = devices.entry(host.clone()).or_insert_with(|| Device::new(&host));
                    dev.via = "mDNS".into();
                    if svc.starts_with("_ipp") {
                        dev.ipp_port = info.get_port();
                        dev.ipp_resource = props.get("rp").map(|v| format!("/{}", v.val_str())).unwrap_or_else(|| "/ipp/print".into());
                        dev.name = info.get_fullname().split('.').next().unwrap_or("").to_string();
                        if let Some(ty) = props.get("ty") {
                            dev.model = ty.val_str().to_string();
                        }
                    } else if svc.starts_with("_uscan") {
                        dev.scan_port = Some(info.get_port());
                        dev.scan_resource = props.get("rs").map(|v| format!("/{}", v.val_str())).unwrap_or_else(|| "/eSCL".into());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = mdns.shutdown();
    devices.into_values().collect()
}

pub fn discover(log: impl Fn(&str) + Copy + Sync) -> Vec<Device> {
    log("Looking for printers via mDNS...");
    let found = discover_mdns(Duration::from_secs(4), log);
    if !found.is_empty() {
        return found;
    }
    log("Nothing on mDNS. Sweeping the local network directly.");
    discover_subnet(log)
}
