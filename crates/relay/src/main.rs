//! `wanderbooks-relay`: run the relay from a configuration file.
//!
//! ```text
//! wanderbooks-relay <config.toml>
//! wanderbooks-relay <config.toml> block <four-word-name>
//! wanderbooks-relay <config.toml> unblock <four-word-name>
//! ```

use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::time::Duration;

use relay::authorities::AuthorityList;
use relay::names::Name;
use relay::registry::Registry;
use relay::{Config, Relay, tls};

fn fail(msg: &str) -> ExitCode {
    eprintln!("wanderbooks-relay: {msg}");
    ExitCode::FAILURE
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        return fail("usage: wanderbooks-relay <config.toml> [block|unblock <name>]");
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return fail(&format!("{path}: {e}")),
    };
    let cfg = match Config::from_toml(&text) {
        Ok(c) => c,
        Err(e) => return fail(&format!("{path}: {e}")),
    };

    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    if let [_, verb @ ("block" | "unblock"), name] = words.as_slice() {
        let Ok(name) = Name::parse(name) else {
            return fail("a name is four words from the list, joined by hyphens");
        };
        let registry = match Registry::open(&cfg.registry_path, 4) {
            Ok(r) => r,
            Err(e) => return fail(&e.to_string()),
        };
        return match registry.set_blocked(&name, *verb == "block").await {
            Ok(true) => {
                println!("{verb}ed {name}");
                ExitCode::SUCCESS
            }
            Ok(false) => fail("no such name"),
            Err(e) => fail(&e.to_string()),
        };
    }
    if args.len() != 1 {
        return fail("usage: wanderbooks-relay <config.toml> [block|unblock <name>]");
    }

    let tls = match tls::server_config_from_files(&cfg.tls_cert, &cfg.tls_key) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let authorities = match &cfg.authorities_path {
        Some(p) => match AuthorityList::from_file(p.clone()) {
            Ok(l) => l,
            Err(e) => return fail(&e.to_string()),
        },
        None => AuthorityList::empty(),
    };
    let relay = match Relay::start(cfg, tls, authorities).await {
        Ok(r) => r,
        Err(e) => return fail(&format!("could not start: {e}")),
    };
    eprintln!(
        "wanderbooks-relay: router on {}, DNS on {} (udp) and {} (tcp)",
        relay.router_addr(),
        relay.dns_udp_addr(),
        relay.dns_tcp_addr()
    );
    let shared = relay.shared().clone();
    let stats = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let m = &shared.metrics;
            eprintln!(
                "lines {} routed {} unknown_or_offline {} shed {} hello_timeouts {} bad_hello {} claims {}",
                m.lines_up.load(Ordering::Relaxed),
                m.routed.load(Ordering::Relaxed),
                m.unknown_or_offline.load(Ordering::Relaxed),
                m.shed.load(Ordering::Relaxed),
                m.hello_timeouts.load(Ordering::Relaxed),
                m.bad_hello.load(Ordering::Relaxed),
                m.claims.load(Ordering::Relaxed),
            );
        }
    });
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("wanderbooks-relay: shutting down");
    stats.abort();
    relay.shutdown().await;
    ExitCode::SUCCESS
}
