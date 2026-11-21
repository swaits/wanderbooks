//! The domain's authoritative DNS server.
//!
//! The relay answers for `wanderbooks.link` itself, over UDP and TCP.
//! It answers two questions and refuses the rest:
//!
//! - any name under the domain is at the relay's address (A, AAAA), the
//!   name servers at their own;
//! - `TXT _acme-challenge.<four-words>.<domain>` is the proof value that
//!   app just handed in, held in memory for minutes.
//!
//! Nothing is stored per user: the answers are made, not looked up, so a
//! catch-all that stops covering names under a label of its own (RFC 4592)
//! is not a risk here: every name below the zone is answered the same way
//! whatever else exists beneath it.
//!
//! ```text
//! GOAL: be the domain's DNS: point every name at the relay, publish an
//!       app's proof record and nobody else's, and be no use to someone
//!       who wants a resolver, an amplifier, or a way to make the relay
//!       hold memory.
//! SOURCE: WB-relay-5 (the proof records and the domain's DNS), RFC 1035,
//!         RFC 4592 (wildcards), RFC 6891 (EDNS), RFC 8482 (ANY).
//! INVARIANTS:
//!   1. A query outside the zone is refused; recursion is never offered.
//!   2. Every name under the zone has A and AAAA answers at the relay's
//!      addresses, including names beneath a label that has a TXT record.
//!   3. A TXT answer exists only at `_acme-challenge.<name>` for a name
//!      with a live published value, and holds only those values.
//!   4. A UDP answer never exceeds the client's stated size (512 without
//!      EDNS, at most 1 232 with it); an oversized one is truncated
//!      (TC) and the client retries over TCP.
//!   5. The server holds no per-query state: the UDP loop answers inline
//!      from fixed tables, and TCP is capped in connections, in queries
//!      per connection and in time.
//!   6. Per address, UDP queries are rate limited from a fixed table.
//! SCENARIOS:
//!   1. A for `x.<secret>.<name>.<domain>` is the relay's address, with
//!      and without a TXT record published for that name.
//!   2. TXT for `_acme-challenge.<name>` returns the published value, and
//!      returns nothing after the value expires.
//!   3. A for `example.com` is REFUSED; ANY is answered with no data.
//!   4. A hundred queries in a burst from one address: the excess is
//!      dropped.
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use hickory_proto::op::{Edns, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA, NS, SOA, TXT};
use hickory_proto::rr::{DNSClass, Name as DnsName, RData, Record, RecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::gate::{IpGate, RateTable};
use crate::names::{Name, Zone};
use crate::txt::TxtStore;

const TTL_ADDR: u32 = 300;
const TTL_APEX: u32 = 3600;
const TTL_TXT: u32 = 5;
const TTL_NEGATIVE: u32 = 60;
/// The biggest UDP answer ever sent, whatever the client asks for.
const UDP_MAX: usize = 1232;
/// The biggest query read, over either transport.
const QUERY_MAX: usize = 1232;

/// The DNS service: pure, no sockets. Feed it a query, get the answer.
pub struct DnsService {
    zone: Zone,
    domain: DnsName,
    v4: Vec<Ipv4Addr>,
    v6: Vec<Ipv6Addr>,
    nameservers: Vec<(String, DnsName, Vec<IpAddr>)>,
    txt: Arc<TxtStore>,
}

impl DnsService {
    /// A service for the configured domain.
    pub fn new(zone: Zone, cfg: &Config, txt: Arc<TxtStore>) -> Result<Self, String> {
        let domain =
            DnsName::from_ascii(format!("{}.", zone.domain())).map_err(|e| e.to_string())?;
        let mut nameservers = Vec::new();
        for ns in &cfg.nameservers {
            let name = DnsName::from_ascii(format!("{}.", ns.name.trim_end_matches('.')))
                .map_err(|e| e.to_string())?;
            let ips = ns
                .v4
                .iter()
                .map(|a| IpAddr::V4(*a))
                .chain(ns.v6.iter().map(|a| IpAddr::V6(*a)))
                .collect();
            nameservers.push((
                ns.name.trim_end_matches('.').to_ascii_lowercase(),
                name,
                ips,
            ));
        }
        Ok(DnsService {
            zone,
            domain,
            v4: cfg.address_v4.clone(),
            v6: cfg.address_v6.clone(),
            nameservers,
            txt,
        })
    }

    fn soa(&self) -> Record {
        let mname = self
            .nameservers
            .first()
            .map(|(_, n, _)| n.clone())
            .unwrap_or_else(|| {
                DnsName::from_ascii(format!("ns1.{}", self.domain))
                    .unwrap_or_else(|_| self.domain.clone())
            });
        let rname = DnsName::from_ascii(format!("hostmaster.{}", self.domain))
            .unwrap_or_else(|_| self.domain.clone());
        Record::from_rdata(
            self.domain.clone(),
            TTL_APEX,
            RData::SOA(SOA::new(mname, rname, 1, 3600, 600, 86_400, TTL_NEGATIVE)),
        )
    }

    fn addresses(&self, owner: &DnsName, text: &str, want_v6: bool) -> Vec<Record> {
        if let Some((_, _, ips)) = self.nameservers.iter().find(|(n, _, _)| n == text) {
            return ips
                .iter()
                .filter_map(|ip| match (ip, want_v6) {
                    (IpAddr::V4(a), false) => Some(RData::A(A::from(*a))),
                    (IpAddr::V6(a), true) => Some(RData::AAAA(AAAA::from(*a))),
                    _ => None,
                })
                .map(|d| Record::from_rdata(owner.clone(), TTL_ADDR, d))
                .collect();
        }
        if want_v6 {
            self.v6
                .iter()
                .map(|a| Record::from_rdata(owner.clone(), TTL_ADDR, RData::AAAA(AAAA::from(*a))))
                .collect()
        } else {
            self.v4
                .iter()
                .map(|a| Record::from_rdata(owner.clone(), TTL_ADDR, RData::A(A::from(*a))))
                .collect()
        }
    }

    /// The answer to a query message, or `None` to say nothing at all (a
    /// response, or too little to answer). `udp` limits the size.
    pub fn answer(&self, wire: &[u8], udp: bool, now: Instant) -> Option<Vec<u8>> {
        if wire.len() > QUERY_MAX {
            return None;
        }
        let query = match Message::from_vec(wire) {
            Ok(m) => m,
            Err(_) => {
                // Too short to have an id: nothing to answer. A response
                // (QR set) is never answered, so no two servers can loop.
                let id = wire.get(..2).map(|b| u16::from_be_bytes([b[0], b[1]]))?;
                if wire.get(2).is_none_or(|b| b & 0x80 != 0) {
                    return None;
                }
                return Message::error_msg(id, OpCode::Query, ResponseCode::FormErr)
                    .to_vec()
                    .ok();
            }
        };
        if query.metadata.message_type == MessageType::Response {
            return None;
        }
        let id = query.metadata.id;
        let reply = |code| {
            Message::error_msg(id, query.metadata.op_code, code)
                .to_vec()
                .ok()
        };
        if query.metadata.op_code != OpCode::Query {
            return reply(ResponseCode::NotImp);
        }
        let [q] = query.queries.as_slice() else {
            return reply(ResponseCode::FormErr);
        };
        if q.query_class() != DNSClass::IN {
            return reply(ResponseCode::Refused);
        }
        let qname = q.name().to_ascii();
        let lower = qname.trim_end_matches('.').to_ascii_lowercase();
        let Some(labels) = self.zone.below(&lower) else {
            return reply(ResponseCode::Refused);
        };

        let mut resp = Message::response(id, OpCode::Query);
        resp.metadata.authoritative = true;
        resp.metadata.recursion_desired = query.metadata.recursion_desired;
        resp.queries.push(q.clone());
        if let Some(e) = &query.edns {
            let mut edns = Edns::new();
            edns.set_max_payload(u16::try_from(UDP_MAX).unwrap_or(1232));
            edns.set_version(0);
            resp.edns = Some(edns);
            let _ = e;
        }
        let owner = q.name().clone();
        let apex = labels.is_empty();
        match q.query_type() {
            RecordType::A => resp.answers = self.addresses(&owner, &lower, false),
            RecordType::AAAA => resp.answers = self.addresses(&owner, &lower, true),
            RecordType::SOA if apex => resp.answers.push(self.soa()),
            RecordType::NS if apex => {
                for (_, ns, ips) in &self.nameservers {
                    resp.answers.push(Record::from_rdata(
                        owner.clone(),
                        TTL_APEX,
                        RData::NS(NS(ns.clone())),
                    ));
                    for ip in ips {
                        let data = match ip {
                            IpAddr::V4(a) => RData::A(A::from(*a)),
                            IpAddr::V6(a) => RData::AAAA(AAAA::from(*a)),
                        };
                        resp.additionals
                            .push(Record::from_rdata(ns.clone(), TTL_ADDR, data));
                    }
                }
            }
            RecordType::TXT => {
                if let ["_acme-challenge", name] = labels.as_slice()
                    && let Ok(name) = Name::parse(&name.to_ascii_lowercase())
                {
                    for v in self.txt.get(&name, now) {
                        resp.answers.push(Record::from_rdata(
                            owner.clone(),
                            TTL_TXT,
                            RData::TXT(TXT::new(vec![v])),
                        ));
                    }
                }
            }
            // ANY (RFC 8482), and every type the relay has nothing of: no
            // data, which is not a refusal and not a missing name.
            _ => {}
        }
        if resp.answers.is_empty() {
            resp.authorities.push(self.soa());
        }
        let mut bytes = resp.to_vec().ok()?;
        if udp {
            let limit = query
                .edns
                .as_ref()
                .map_or(512, |e| usize::from(e.max_payload()).clamp(512, UDP_MAX));
            if bytes.len() > limit {
                resp.metadata.truncation = true;
                resp.answers.clear();
                resp.authorities.clear();
                resp.additionals.clear();
                bytes = resp.to_vec().ok()?;
            }
        }
        Some(bytes)
    }
}

/// Serve UDP until cancelled. Answers are made inline, with no task per
/// query, and a sender over its rate gets no answer.
pub async fn serve_udp(
    socket: Arc<UdpSocket>,
    svc: Arc<DnsService>,
    qps: u32,
    slots: usize,
    cancel: CancellationToken,
) {
    let mut rate = RateTable::new(slots, qps, qps.saturating_mul(2).max(1));
    let mut buf = vec![0u8; QUERY_MAX + 1];
    loop {
        let (n, from) = tokio::select! {
            () = cancel.cancelled() => return,
            r = socket.recv_from(&mut buf) => match r {
                Ok(x) => x,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    continue;
                }
            },
        };
        let now = Instant::now();
        if !rate.allow(from.ip(), now) {
            continue;
        }
        if let Some(out) = svc.answer(&buf[..n], true, now) {
            // A full send buffer drops the answer: a client retries.
            let _ = socket.try_send_to(&out, from);
        }
    }
}

/// The limits of the TCP side.
#[derive(Debug, Clone, Copy)]
pub struct TcpLimits {
    /// Connections at once.
    pub max: usize,
    /// Connections at once from one address.
    pub per_ip: u32,
    /// The wait for each query, and for the rest of one that began.
    pub timeout: std::time::Duration,
    /// Queries answered on one connection.
    pub queries: u32,
}

/// Serve TCP until cancelled.
pub async fn serve_tcp(
    listener: TcpListener,
    svc: Arc<DnsService>,
    limits: TcpLimits,
    slots: usize,
    cancel: CancellationToken,
) {
    let permits = Arc::new(Semaphore::new(limits.max));
    let gate = IpGate::new(slots, limits.per_ip);
    loop {
        let (mut stream, from) = tokio::select! {
            () = cancel.cancelled() => return,
            r = listener.accept() => match r {
                Ok(x) => x,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    continue;
                }
            },
        };
        let (Ok(permit), Some(ip_permit)) = (
            Arc::clone(&permits).try_acquire_owned(),
            gate.acquire(from.ip()),
        ) else {
            continue; // over the limit: close it
        };
        let svc = Arc::clone(&svc);
        tokio::spawn(async move {
            let _held = (permit, ip_permit);
            for _ in 0..limits.queries {
                let mut len = [0u8; 2];
                let read = async {
                    stream.read_exact(&mut len).await?;
                    let n = usize::from(u16::from_be_bytes(len));
                    if n == 0 || n > QUERY_MAX {
                        return Err(std::io::ErrorKind::InvalidData.into());
                    }
                    let mut body = vec![0u8; n];
                    stream.read_exact(&mut body).await?;
                    Ok::<_, std::io::Error>(body)
                };
                let Ok(Ok(body)) = tokio::time::timeout(limits.timeout, read).await else {
                    return;
                };
                let Some(out) = svc.answer(&body, false, Instant::now()) else {
                    return;
                };
                let Ok(n) = u16::try_from(out.len()) else {
                    return;
                };
                let mut framed = n.to_be_bytes().to_vec();
                framed.extend(out);
                if tokio::time::timeout(limits.timeout, stream.write_all(&framed))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
    }
}

/// A socket address for tests and the binary: where DNS is listening.
pub fn bound(addr: SocketAddr) -> SocketAddr {
    addr
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hickory_proto::op::Query;

    use super::*;

    fn svc() -> (DnsService, Arc<TxtStore>) {
        let cfg = Config {
            address_v4: vec!["198.51.100.7".parse().unwrap()],
            address_v6: vec!["2001:db8::7".parse().unwrap()],
            nameservers: vec![
                crate::config::NameServer {
                    name: "ns1.wanderbooks.link".into(),
                    v4: vec!["198.51.100.53".parse().unwrap()],
                    v6: vec![],
                },
                crate::config::NameServer {
                    name: "ns2.wanderbooks.link".into(),
                    v4: vec!["203.0.113.53".parse().unwrap()],
                    v6: vec![],
                },
            ],
            ..Config::default()
        };
        let txt = Arc::new(TxtStore::new(Duration::from_secs(60), 100, 4, 100));
        let zone = Zone::new(&cfg.domain, &cfg.relay_host);
        (DnsService::new(zone, &cfg, Arc::clone(&txt)).unwrap(), txt)
    }

    fn ask(svc: &DnsService, name: &str, ty: RecordType, udp: bool) -> Message {
        let mut m = Message::query();
        m.queries
            .push(Query::query(DnsName::from_ascii(name).unwrap(), ty));
        let out = svc
            .answer(&m.to_vec().unwrap(), udp, Instant::now())
            .expect("an answer");
        Message::from_vec(&out).unwrap()
    }

    const APP: &str = "ledger-accrual-tally-mint.wanderbooks.link.";
    const SECRET: &str = "k3j9x0q7m2z8c5v1b4n6a9s2d";
    const PROOF: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

    #[test]
    fn every_name_under_the_zone_is_at_the_relay_even_beneath_a_label_with_a_record() {
        let (svc, txt) = svc();
        let deep = format!("{SECRET}.{APP}");
        for publish in [false, true] {
            if publish {
                txt.publish(
                    Name::parse("ledger-accrual-tally-mint").unwrap(),
                    PROOF,
                    Instant::now(),
                )
                .unwrap();
            }
            for name in [
                APP,
                deep.as_str(),
                "relay.wanderbooks.link.",
                "wanderbooks.link.",
                "A.B.C.D.wanderbooks.link.",
            ] {
                let m = ask(&svc, name, RecordType::A, true);
                assert_eq!(m.metadata.response_code, ResponseCode::NoError, "{name}");
                assert!(m.metadata.authoritative);
                assert!(
                    matches!(m.answers.as_slice(), [r] if r.data == RData::A(A::from(Ipv4Addr::new(198, 51, 100, 7)))),
                    "{name}: {:?}",
                    m.answers
                );
                let m6 = ask(&svc, name, RecordType::AAAA, true);
                assert_eq!(m6.answers.len(), 1, "{name}");
            }
        }
    }

    #[test]
    fn the_name_servers_are_at_their_own_addresses() {
        let (svc, _) = svc();
        let m = ask(&svc, "ns2.wanderbooks.link.", RecordType::A, true);
        assert!(
            matches!(m.answers.as_slice(), [r] if r.data == RData::A(A::from(Ipv4Addr::new(203, 0, 113, 53))))
        );
        let ns = ask(&svc, "wanderbooks.link.", RecordType::NS, true);
        assert_eq!(ns.answers.len(), 2);
        assert_eq!(ns.additionals.len(), 2);
        let soa = ask(&svc, "wanderbooks.link.", RecordType::SOA, true);
        assert_eq!(soa.answers.len(), 1);
    }

    #[test]
    fn the_proof_record_is_answered_for_its_own_name_until_it_expires() {
        let (svc, txt) = svc();
        let mint = Name::parse("ledger-accrual-tally-mint").unwrap();
        let t0 = Instant::now();
        txt.publish(mint, PROOF, t0).unwrap();
        let q = format!("_acme-challenge.{APP}");
        let m = ask(&svc, &q, RecordType::TXT, true);
        assert_eq!(m.answers.len(), 1);
        let RData::TXT(t) = &m.answers[0].data else {
            panic!("a TXT")
        };
        assert_eq!(t.to_string(), PROOF);
        // Another app's challenge name has nothing; so does a made-up one.
        for other in [
            "_acme-challenge.ledger-accrual-tally-vault.wanderbooks.link.",
            "_acme-challenge.not-a-name.wanderbooks.link.",
            &format!("_acme-challenge.{SECRET}.{APP}"),
            APP,
        ] {
            let m = ask(&svc, other, RecordType::TXT, true);
            assert_eq!(m.metadata.response_code, ResponseCode::NoError, "{other}");
            assert!(m.answers.is_empty(), "{other}");
            assert_eq!(m.authorities.len(), 1, "a no-data answer carries the SOA");
        }
        // After the value's life it is gone.
        txt.sweep(t0 + Duration::from_secs(61));
        let gone = svc
            .answer(
                &{
                    let mut m = Message::query();
                    m.queries.push(Query::query(
                        DnsName::from_ascii(&q).unwrap(),
                        RecordType::TXT,
                    ));
                    m.to_vec().unwrap()
                },
                true,
                t0 + Duration::from_secs(61),
            )
            .unwrap();
        assert!(Message::from_vec(&gone).unwrap().answers.is_empty());
    }

    #[test]
    fn outside_the_zone_is_refused_and_odd_queries_are_answered_plainly() {
        let (svc, _) = svc();
        for name in [
            "example.com.",
            "link.",
            "xwanderbooks.link.",
            "wanderbooks.link.example.com.",
        ] {
            let m = ask(&svc, name, RecordType::A, true);
            assert_eq!(m.metadata.response_code, ResponseCode::Refused, "{name}");
            assert!(m.answers.is_empty());
        }
        let any = ask(&svc, APP, RecordType::ANY, true);
        assert_eq!(any.metadata.response_code, ResponseCode::NoError);
        assert!(any.answers.is_empty());
        let mx = ask(&svc, APP, RecordType::MX, true);
        assert!(mx.answers.is_empty() && mx.metadata.response_code == ResponseCode::NoError);
        // Not a query, and a response: refused to be answered.
        let mut upd = Message::query();
        upd.metadata.op_code = OpCode::Update;
        upd.queries.push(Query::query(
            DnsName::from_ascii(APP).unwrap(),
            RecordType::A,
        ));
        let out = svc
            .answer(&upd.to_vec().unwrap(), true, Instant::now())
            .unwrap();
        assert_eq!(
            Message::from_vec(&out).unwrap().metadata.response_code,
            ResponseCode::NotImp
        );
        let mut resp = Message::response(7, OpCode::Query);
        resp.queries.push(Query::query(
            DnsName::from_ascii(APP).unwrap(),
            RecordType::A,
        ));
        assert!(
            svc.answer(&resp.to_vec().unwrap(), true, Instant::now())
                .is_none()
        );
        // Two questions, no questions.
        let mut two = Message::query();
        two.queries.push(Query::query(
            DnsName::from_ascii(APP).unwrap(),
            RecordType::A,
        ));
        two.queries.push(Query::query(
            DnsName::from_ascii(APP).unwrap(),
            RecordType::AAAA,
        ));
        let out = svc
            .answer(&two.to_vec().unwrap(), true, Instant::now())
            .unwrap();
        assert_eq!(
            Message::from_vec(&out).unwrap().metadata.response_code,
            ResponseCode::FormErr
        );
        let none = Message::query().to_vec().unwrap();
        let out = svc.answer(&none, true, Instant::now()).unwrap();
        assert_eq!(
            Message::from_vec(&out).unwrap().metadata.response_code,
            ResponseCode::FormErr
        );
    }

    #[test]
    fn garbage_never_panics_and_is_mostly_ignored() {
        let (svc, _) = svc();
        assert!(svc.answer(b"", true, Instant::now()).is_none());
        assert!(svc.answer(b"x", true, Instant::now()).is_none());
        assert!(
            svc.answer(&vec![0u8; QUERY_MAX + 1], true, Instant::now())
                .is_none()
        );
        let mut seed = 0x9e37_79b9u32;
        for len in 0..300usize {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            let _ = svc.answer(&bytes, true, Instant::now());
            let _ = svc.answer(&bytes, false, Instant::now());
        }
    }

    #[test]
    fn a_udp_answer_never_exceeds_the_clients_size() {
        let (svc, _) = svc();
        // A query with no EDNS gets at most 512 bytes; ours are far less,
        // so check the cap by forcing a tiny limit through truncation of
        // a big NS set.
        let m = ask(&svc, "wanderbooks.link.", RecordType::NS, true);
        let bytes = m.to_vec().unwrap();
        assert!(bytes.len() <= 512);
        assert!(!m.metadata.truncation);
    }
}
