//! The relay for `wanderbooks.link`.
//!
//! A person runs the Wanderbooks desktop app at home. Web assistants can
//! only call servers on the public internet. The app dials out to this
//! relay and holds one multiplexed line open; the relay routes an
//! assistant's incoming TLS connection, by its server name, down that
//! line to the app. The relay never holds the app's private key and
//! cannot read the traffic: it forwards encrypted bytes. It is also the
//! domain's authoritative DNS server, which lets an app prove its own
//! name to a certificate authority and no other.
//!
//! The crate depends on no `ledger-*` crate. The modules [`proto`],
//! [`names`] and [`client`] are the app's side of the protocol too.
//!
//! ```text
//! GOAL: let a web assistant reach an app behind a home network, with
//!       the relay unable to read anything, unable to be made to hold
//!       unbounded memory, and unable to give a stranger an app's name
//!       or a certificate for it.
//! SOURCE: WB-relay-1 (the router), WB-relay-2 (the line and its proof),
//!         WB-relay-3 (names), WB-relay-4 (the registry), WB-relay-5
//!         (proof records, DNS and authorities), WB-relay-6 (the
//!         caller's address), WB-relay-7 (bounds); D-80, D-82, D-85, D-87.
//! INVARIANTS:
//!   1. No unsafe code anywhere in the crate.
//!   2. The relay holds no key or certificate for any app and never
//!      decrypts an app's traffic.
//!   3. The only durable state is the registry (name, key, created,
//!      blocked); proof records live minutes, in memory.
//! SCENARIOS:
//!   1. See `tests/relay.rs`: a real TLS client reaches a real TLS
//!      server through the relay, and the relay never sees plaintext.
//! ```

#![forbid(unsafe_code)]

pub mod authorities;
pub mod client;
pub mod config;
pub mod dns;
pub mod front;
pub mod gate;
pub mod hello;
pub mod mux;
pub mod names;
pub mod proto;
pub mod registry;
pub mod server;
pub mod splice;
pub mod tls;
pub mod txt;

pub use config::{Config, Limits};
pub use server::Relay;
