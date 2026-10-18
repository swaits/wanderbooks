//! `ledger-mcp`: the books server over stdio (or `--http ADDR`). The same
//! server is `wanderbooks dev-stdio`; both call [`ledger_mcp::run`].

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match ledger_mcp::run("ledger-mcp", std::env::args().skip(1)) {
        Err(e) => match e.downcast_ref::<ledger_mcp::Exit>() {
            Some(exit) => std::process::exit(exit.0),
            None => Err(e),
        },
        done => done,
    }
}
