//! `ledger-gui`: the read-only, live viewer. The same viewer is
//! `wanderbooks gui`; both call [`ledger_gui::run`].

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match ledger_gui::run("ledger-gui", std::env::args().skip(1)) {
        Err(e) => match e.downcast_ref::<ledger_gui::Exit>() {
            Some(exit) => std::process::exit(exit.0),
            None => Err(e),
        },
        done => done,
    }
}
