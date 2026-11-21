//! What an idle line costs the relay, measured.
//!
//! Section 7 of the plan lists this as unknown, and it sizes the host.
//! The bench runs the relay in this process and the apps in a child
//! process (so the relay's resident memory is the relay's alone), brings
//! up N lines, and reads the relay's `VmRSS` before and after. It then
//! drops every line and brings them all up again twice more (churn), and
//! checks that memory does not grow.
//!
//! It needs one descriptor per line in each process, so it is ignored in
//! a normal run:
//!
//! ```text
//! ulimit -n 65536
//! TMPDIR=/dev/shm RELAY_IDLE_LINES=20000 cargo test --release \
//!     -p wanderbooks-relay --test memory idle_line_memory -- --ignored --nocapture
//! ```
//!
//! `TMPDIR=/dev/shm` keeps the registry off the disk: every claim of a
//! name is fsynced (that is the point of the registry), and 20 000 of
//! them take minutes on a disk. Measured 2026-11-21 on this machine,
//! release build: 20 000 lines, 15.2 KiB of the relay's resident memory
//! per idle line, and 15.3 KiB after two rounds of churn (flat).
//!
//! Linux only (it reads /proc).
#![allow(missing_docs)]
#![cfg(target_os = "linux")]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

use common::*;
use relay::Limits;
use relay::client::{self, LineOptions};
use relay::names::Name;
use relay::proto::AppKey;
use tokio::sync::Semaphore;

fn rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .expect("VmRSS")
}

fn lines_wanted() -> usize {
    std::env::var("RELAY_IDLE_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

/// The child: holds N lines, on request drops and remakes them.
/// Run only by `idle_line_memory`, which sets RELAY_HERD.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a helper process for idle_line_memory"]
async fn herd_child() {
    let Ok(spec) = std::env::var("RELAY_HERD") else {
        return;
    };
    let mut parts = spec.split('|');
    let addr: std::net::SocketAddr = parts.next().unwrap().parse().unwrap();
    let count: usize = parts.next().unwrap().parse().unwrap();
    let ca_path = parts.next().unwrap();
    let pem = std::fs::read_to_string(ca_path).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    for c in
        <rustls::pki_types::CertificateDer as rustls::pki_types::pem::PemObject>::pem_slice_iter(
            pem.as_bytes(),
        )
    {
        roots.add(c.unwrap()).unwrap();
    }
    let tls = relay::tls::client_config(roots).unwrap();
    let ids: Vec<(Name, Arc<AppKey>)> = (0..count)
        .map(|_| (Name::random(), Arc::new(AppKey::generate().0)))
        .collect();

    let mut claim = true;
    let mut round = 0;
    let stdin = std::io::stdin();
    loop {
        let gate = Arc::new(Semaphore::new(64));
        let mut tasks = Vec::new();
        for (name, key) in &ids {
            let (name, key, tls, gate) =
                (*name, Arc::clone(key), Arc::clone(&tls), Arc::clone(&gate));
            tasks.push(tokio::spawn(async move {
                let _p = gate.acquire().await.unwrap();
                client::connect(
                    addr,
                    tls,
                    RELAY_HOST,
                    name,
                    &key,
                    claim,
                    LineOptions::default(),
                )
                .await
                .expect("a line")
            }));
        }
        let mut lines = Vec::with_capacity(count);
        for t in tasks {
            lines.push(t.await.unwrap());
        }
        claim = false;
        round += 1;
        println!("READY {round}");
        std::io::stdout().flush().unwrap();
        let mut cmd = String::new();
        if stdin.lock().read_line(&mut cmd).unwrap() == 0 || cmd.trim() == "stop" {
            return;
        }
        drop(lines); // the lines close, then the loop brings them back
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a descriptor per line: see the module comment"]
async fn idle_line_memory() {
    let n = lines_wanted();
    let env = start(Limits {
        max_connections: 1_000_000,
        max_connections_per_ip: 1_000_000,
        max_pending_per_ip: 10_000,
        max_pending_hellos: 10_000,
        max_handshakes: 1024,
        max_lines: n + 10,
        line_idle_ms: 3_600_000,
        claims_per_window: u32::try_from(n + 10).unwrap(),
        claims_per_ip: u32::try_from(n + 10).unwrap(),
        ..quick_limits()
    })
    .await;
    // Warm up: one line, so one-time allocations are not counted.
    let (k, _) = AppKey::generate();
    env.connect(Name::random(), &k, true)
        .await
        .unwrap()
        .close()
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let before = rss_kib();

    let ca = env.dir.path().join("ca.pem");
    std::fs::write(&ca, env.relay_ca.pem()).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "herd_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(
            "RELAY_HERD",
            format!("{}|{n}|{}", env.router(), ca.display()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let started = std::time::Instant::now();
    let mut held = Vec::new();
    for round in 1..=3 {
        let mut line = String::new();
        loop {
            line.clear();
            assert!(
                out.read_line(&mut line).unwrap() > 0,
                "the child ended early"
            );
            if line.trim().ends_with(&format!("READY {round}")) {
                break;
            }
        }
        // Let the sockets settle and the allocator hand back what it can.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let up = env.relay.shared().lines.len();
        assert_eq!(up, n, "every line is up");
        let rss = rss_kib();
        held.push(rss);
        eprintln!(
            "round {round}: {up} lines up, relay RSS {rss} KiB, {:.2} KiB per line over the {before} KiB baseline ({:?} so far)",
            (rss.saturating_sub(before)) as f64 / n as f64,
            started.elapsed()
        );
        writeln!(stdin, "{}", if round == 3 { "stop" } else { "again" }).unwrap();
        stdin.flush().unwrap();
        if round < 3 {
            // The child drops its lines; wait for the relay to see them go.
            eventually_async(|| env.relay.shared().lines.len() < n).await;
        }
    }
    let _ = child.wait();
    let per_line = held[0].saturating_sub(before) as f64 / n as f64;
    eprintln!("idle line: {per_line:.2} KiB in the relay's resident memory (N = {n})");
    // Churn leaves memory flat: the third round is within 10% of the
    // first, over the baseline.
    let grown = held[2].saturating_sub(held[0]);
    assert!(
        (grown as f64) < 0.10 * (held[0].saturating_sub(before) as f64) + 2048.0,
        "memory grew {grown} KiB over two rounds of churn: {held:?}"
    );
    // And a ceiling, generous: a line must not cost 64 KiB.
    assert!(per_line < 64.0, "{per_line} KiB per idle line");
}

async fn eventually_async<F: FnMut() -> bool>(mut f: F) {
    for _ in 0..1200 {
        if f() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the lines never went away");
}
