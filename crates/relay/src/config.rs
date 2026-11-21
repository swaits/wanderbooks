//! The relay's settings, and every limit it enforces.
//!
//! Every limit is a number here with a default and a reason; nothing in
//! the relay is unbounded. The defaults are sized for hundreds of
//! thousands of idle apps on one modest host.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// A name server of the domain, and where it is.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameServer {
    /// Its host name (`ns1.wanderbooks.link`).
    pub name: String,
    /// Its IPv4 addresses.
    #[serde(default)]
    pub v4: Vec<Ipv4Addr>,
    /// Its IPv6 addresses.
    #[serde(default)]
    pub v6: Vec<Ipv6Addr>,
}

/// Every limit and timeout. Times are in milliseconds.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Apps connected at once. Admission stops here.
    pub max_lines: usize,
    /// Connections open at once, all kinds (hello stage, line handshake,
    /// callers being spliced, apps' lines).
    pub max_connections: usize,
    /// Connections that have not yet sent a whole ClientHello.
    pub max_pending_hellos: usize,
    /// Of those, from one address (an IPv4 address, an IPv6 /64). Strict:
    /// this is the slow-loris limit.
    pub max_pending_per_ip: u32,
    /// Of those, from one network (an IPv4 /24, an IPv6 /48).
    pub max_pending_per_net: u32,
    /// Hello places kept back for addresses that held a proven line
    /// lately, used only when `max_pending_hellos` is full. The server
    /// name is not known before the hello is read, so the reserve is for
    /// who is calling, not for what is asked.
    pub hello_reserve: usize,
    /// Bits in each generation of the filter of addresses that held a
    /// proven line (a power of two).
    pub known_bits: usize,
    /// An address that held a proven line counts as known for one to two
    /// of these.
    pub known_period_ms: u64,
    /// Connections from one address, all stages. Generous: the AI vendors'
    /// callers share a few addresses between all their users.
    pub max_connections_per_ip: u32,
    /// How long a peer has to send a whole ClientHello, from connecting.
    pub hello_timeout_ms: u64,
    /// Line handshakes (TLS and proof) in progress at once.
    pub max_handshakes: usize,
    /// Of those, from one address (an IPv4 address, an IPv6 /64), held
    /// until the verdict.
    pub max_handshakes_per_ip: u32,
    /// Of those, from one network (an IPv4 /24, an IPv6 /48).
    pub max_handshakes_per_net: u32,
    /// How long a line's TLS handshake and proof may take in all.
    pub handshake_timeout_ms: u64,
    /// How long to wait for the app to open a stream for a caller.
    pub open_stream_timeout_ms: u64,
    /// Caller streams open on one app's line at once.
    pub streams_per_app: u32,
    /// Control streams (requests from the app) in progress at once, per line.
    pub control_streams_per_app: u32,
    /// The copy buffer, per direction, per spliced connection, in bytes.
    pub copy_buffer: usize,
    /// A spliced connection with no byte either way this long is closed.
    pub stream_idle_ms: u64,
    /// The slowest a spliced connection may take bytes, per second: a
    /// write of `n` bytes must finish within `write_stall_ms` plus
    /// `n` over this, or the connection is closed.
    pub min_stream_bytes_per_sec: u32,
    /// A spliced connection's longest life.
    pub stream_max_age_ms: u64,
    /// The grace on every write, before the rate above is asked for: a
    /// slow reader is closed, not buffered for.
    pub write_stall_ms: u64,
    /// A line from which nothing has been read this long is dropped. The
    /// app sends a keep-alive well inside this.
    pub line_idle_ms: u64,
    /// A control request, read to reply, must finish in this long.
    pub control_timeout_ms: u64,
    /// How long a published proof record lives.
    pub txt_ttl_ms: u64,
    /// Proof records held at most, over all apps.
    pub txt_max_entries: usize,
    /// Values one name may hold at once (the wildcard and the apex).
    pub txt_per_name: usize,
    /// Publishes one name may make in an hour.
    pub txt_publishes_per_hour: u32,
    /// New names that may be claimed in one window, over all apps: a
    /// safety ceiling on the registry, far above real use. It is not the
    /// certificate authority's budget (see `cert_publishes_per_window`).
    pub claims_per_window: u32,
    /// The window for `claims_per_window`.
    pub claim_window_ms: u64,
    /// New names one address (an IPv4 address, an IPv6 /64) may claim in
    /// `claim_ip_window_ms`.
    pub claims_per_ip: u32,
    /// New names one network (an IPv4 /24, an IPv6 /48) may claim in the
    /// same window.
    pub claims_per_net: u32,
    /// The window for `claims_per_ip` and `claims_per_net`.
    pub claim_ip_window_ms: u64,
    /// Names that may first publish a proof record in one window, over all
    /// apps: the certificate authority's new-certificate limit (50 a
    /// week). A name is charged once, the first time it publishes; a
    /// renewal is not a new name and is not charged.
    pub cert_publishes_per_window: u32,
    /// The window for the certificate budget: a week.
    pub cert_window_ms: u64,
    /// First publishes one address (an IPv4 address, an IPv6 /64) may
    /// make in `cert_window_ms`.
    pub cert_publishes_per_ip: u32,
    /// First publishes one network (an IPv4 /24, an IPv6 /48) may make in
    /// the same window.
    pub cert_publishes_per_net: u32,
    /// How often the running relay asks the registry which names are
    /// blocked and closes their lines: a block takes effect within this.
    pub block_check_ms: u64,
    /// Registry requests queued at most; past this the relay says busy.
    pub registry_queue: usize,
    /// Slots in each fixed per-address table (a power of two).
    pub ip_slots: usize,
    /// DNS queries per second one address may send, sustained. Past it a
    /// query is answered "truncated", which sends a real resolver to TCP.
    pub dns_qps_per_ip: u32,
    /// DNS queries per second answered over UDP in all: past it a query
    /// gets no answer at all.
    pub dns_qps_global: u32,
    /// TCP DNS connections open at once.
    pub dns_tcp_max: usize,
    /// TCP DNS connections from one address.
    pub dns_tcp_per_ip: u32,
    /// A TCP DNS connection's wait for its next query.
    pub dns_tcp_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_lines: 500_000,
            max_connections: 1_000_000,
            max_pending_hellos: 4_096,
            max_pending_per_ip: 32,
            max_pending_per_net: 256,
            hello_reserve: 512,
            known_bits: 1 << 23,
            known_period_ms: 12 * 3_600_000,
            max_connections_per_ip: 20_000,
            hello_timeout_ms: 8_000,
            max_handshakes: 1_024,
            max_handshakes_per_ip: 4,
            max_handshakes_per_net: 64,
            handshake_timeout_ms: 10_000,
            open_stream_timeout_ms: 5_000,
            streams_per_app: 8,
            control_streams_per_app: 2,
            copy_buffer: 8 * 1024,
            stream_idle_ms: 10 * 60_000,
            stream_max_age_ms: 6 * 3_600_000,
            min_stream_bytes_per_sec: 2_048,
            write_stall_ms: 5_000,
            line_idle_ms: 180_000,
            control_timeout_ms: 10_000,
            txt_ttl_ms: 10 * 60_000,
            txt_max_entries: 100_000,
            txt_per_name: 4,
            txt_publishes_per_hour: 12,
            claims_per_window: 5_000,
            claim_window_ms: 24 * 3_600_000,
            claims_per_ip: 5,
            claims_per_net: 20,
            claim_ip_window_ms: 24 * 3_600_000,
            cert_publishes_per_window: 50,
            cert_window_ms: 7 * 24 * 3_600_000,
            cert_publishes_per_ip: 3,
            cert_publishes_per_net: 10,
            block_check_ms: 10_000,
            registry_queue: 256,
            ip_slots: 1 << 16,
            dns_qps_per_ip: 20,
            dns_qps_global: 5_000,
            dns_tcp_max: 512,
            dns_tcp_per_ip: 8,
            dns_tcp_timeout_ms: 5_000,
        }
    }
}

macro_rules! durations {
    ($($name:ident => $field:ident),* $(,)?) => {
        impl Limits {
            $(
                #[doc = concat!("`", stringify!($field), "` as a duration.")]
                pub fn $name(&self) -> Duration { Duration::from_millis(self.$field) }
            )*
        }
    };
}

durations! {
    hello_timeout => hello_timeout_ms,
    handshake_timeout => handshake_timeout_ms,
    open_stream_timeout => open_stream_timeout_ms,
    stream_idle => stream_idle_ms,
    stream_max_age => stream_max_age_ms,
    write_stall => write_stall_ms,
    line_idle => line_idle_ms,
    control_timeout => control_timeout_ms,
    txt_ttl => txt_ttl_ms,
    claim_window => claim_window_ms,
    claim_ip_window => claim_ip_window_ms,
    cert_window => cert_window_ms,
    known_period => known_period_ms,
    block_check => block_check_ms,
    dns_tcp_timeout => dns_tcp_timeout_ms,
}

/// The relay's configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The domain the relay serves (`wanderbooks.link`).
    pub domain: String,
    /// The host an app's line dials (`relay.wanderbooks.link`).
    pub relay_host: String,
    /// Where the router listens (443 in production).
    pub router_addr: SocketAddr,
    /// Where DNS listens, UDP and TCP (53 in production).
    pub dns_addr: SocketAddr,
    /// The registry's file: the one durable table (D-85).
    pub registry_path: PathBuf,
    /// The relay's own certificate chain, PEM, for `relay_host`.
    pub tls_cert: PathBuf,
    /// Its private key, PEM.
    pub tls_key: PathBuf,
    /// The certificate authorities, served to apps; read again when it
    /// changes. See [`crate::authorities`].
    pub authorities_path: Option<PathBuf>,
    /// IPv4 addresses every name under the domain points at.
    pub address_v4: Vec<Ipv4Addr>,
    /// IPv6 addresses every name under the domain points at.
    pub address_v6: Vec<Ipv6Addr>,
    /// The domain's name servers, at least two in production.
    pub nameservers: Vec<NameServer>,
    /// Limits and timeouts.
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            domain: "wanderbooks.link".into(),
            relay_host: "relay.wanderbooks.link".into(),
            router_addr: SocketAddr::from(([0, 0, 0, 0], 443)),
            dns_addr: SocketAddr::from(([0, 0, 0, 0], 53)),
            registry_path: PathBuf::from("registry.sqlite3"),
            tls_cert: PathBuf::from("relay-cert.pem"),
            tls_key: PathBuf::from("relay-key.pem"),
            authorities_path: None,
            address_v4: Vec::new(),
            address_v6: Vec::new(),
            nameservers: Vec::new(),
            limits: Limits::default(),
        }
    }
}

impl Config {
    /// Read a TOML configuration.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load_and_unknown_keys_are_refused() {
        let c = Config::from_toml("").expect("empty is the defaults");
        assert_eq!(c.limits.streams_per_app, 8);
        let c = Config::from_toml("[limits]\nstreams_per_app = 4\n").unwrap();
        assert_eq!(c.limits.streams_per_app, 4);
        assert!(Config::from_toml("bogus = 1").is_err());
        assert!(Config::from_toml("[limits]\nbogus = 1").is_err());
    }
}
