//! The host's and bridge's timings, which tests shorten (`--test-knobs`).
//!
//! Every mode that starts another passes its knobs on, so a test sets
//! them once, on the holder it starts. Nothing else sets them: the
//! defaults are the plan's (3.9).

use std::time::Duration;

/// The timings in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    /// How long the host waits after the last lease ends before it
    /// closes the books (plan: 10 s).
    pub close_after: Duration,
    /// How often the host looks whether its executable was replaced.
    pub exe_poll: Duration,
    /// The first wait before a holder starts a failed host again; it
    /// doubles each time (plan: 1, 2, 4 … 60 s, five tries).
    pub backoff: Duration,
    /// How long the host waits before starting the server (tests of a
    /// slow start only; 0 otherwise).
    pub server_delay: Duration,
    /// How long the bridge waits for the server before answering
    /// `initialize` itself (plan: 5 s).
    pub init_wait: Duration,
    /// How long the bridge holds a call while the books open (plan: 60 s).
    pub queue_wait: Duration,
    /// How long a command waits for the books to open.
    pub open_wait: Duration,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            close_after: Duration::from_secs(10),
            exe_poll: Duration::from_secs(5),
            backoff: Duration::from_secs(1),
            server_delay: Duration::ZERO,
            init_wait: Duration::from_secs(5),
            queue_wait: Duration::from_secs(60),
            open_wait: Duration::from_secs(120),
        }
    }
}

/// The flag that carries them.
pub const FLAG: &str = "--test-knobs";

impl Knobs {
    /// Read `name=millis,...` (any subset; the rest stay default).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut k = Self::default();
        for pair in text.split(',').filter(|p| !p.is_empty()) {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("{FLAG}: {pair:?} isn't name=milliseconds"))?;
            let ms: u64 = value
                .parse()
                .map_err(|_| format!("{FLAG}: {value:?} isn't a number of milliseconds"))?;
            let d = Duration::from_millis(ms);
            match name {
                "close_after" => k.close_after = d,
                "exe_poll" => k.exe_poll = d,
                "backoff" => k.backoff = d,
                "server_delay" => k.server_delay = d,
                "init_wait" => k.init_wait = d,
                "queue_wait" => k.queue_wait = d,
                "open_wait" => k.open_wait = d,
                other => return Err(format!("{FLAG}: there is no knob {other:?}")),
            }
        }
        Ok(k)
    }

    /// The flag's value that gives these knobs, or `None` for the
    /// defaults (then nothing is passed on).
    pub fn to_arg(&self) -> Option<String> {
        let d = Self::default();
        let mut parts = Vec::new();
        for (name, now, default) in [
            ("close_after", self.close_after, d.close_after),
            ("exe_poll", self.exe_poll, d.exe_poll),
            ("backoff", self.backoff, d.backoff),
            ("server_delay", self.server_delay, d.server_delay),
            ("init_wait", self.init_wait, d.init_wait),
            ("queue_wait", self.queue_wait, d.queue_wait),
            ("open_wait", self.open_wait, d.open_wait),
        ] {
            if now != default {
                parts.push(format!("{name}={}", now.as_millis()));
            }
        }
        (!parts.is_empty()).then(|| parts.join(","))
    }

    /// The arguments that pass these knobs on to another mode.
    pub fn args(&self) -> Vec<String> {
        match self.to_arg() {
            Some(v) => vec![FLAG.to_string(), v],
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_plans() {
        let k = Knobs::default();
        assert_eq!(k.close_after, Duration::from_secs(10));
        assert_eq!(k.backoff, Duration::from_secs(1));
        assert_eq!(k.init_wait, Duration::from_secs(5));
        assert_eq!(k.queue_wait, Duration::from_secs(60));
        assert_eq!(k.exe_poll, Duration::from_secs(5));
        assert_eq!(k.server_delay, Duration::ZERO);
        assert_eq!(k.open_wait, Duration::from_secs(120));
        assert_eq!(k.to_arg(), None);
        assert!(k.args().is_empty());
        assert_eq!(Knobs::parse("").unwrap(), k);
    }

    #[test]
    fn knobs_pass_on_exactly() {
        let k = Knobs::parse(
            "close_after=500,backoff=20,server_delay=3000,init_wait=100,queue_wait=900,exe_poll=50,open_wait=70",
        )
        .unwrap();
        assert_eq!(k.close_after, Duration::from_millis(500));
        assert_eq!(k.backoff, Duration::from_millis(20));
        assert_eq!(k.server_delay, Duration::from_millis(3000));
        assert_eq!(k.init_wait, Duration::from_millis(100));
        assert_eq!(k.queue_wait, Duration::from_millis(900));
        assert_eq!(k.exe_poll, Duration::from_millis(50));
        assert_eq!(k.open_wait, Duration::from_millis(70));
        let args = k.args();
        assert_eq!(args[0], FLAG);
        assert_eq!(Knobs::parse(&args[1]).unwrap(), k);
        let one = Knobs::parse("close_after=1").unwrap();
        assert_eq!(one.to_arg().as_deref(), Some("close_after=1"));
    }

    #[test]
    fn a_bad_knob_is_refused() {
        for bad in ["close_after", "close_after=x", "nope=1", "close_after=-1"] {
            assert!(Knobs::parse(bad).is_err(), "{bad}");
        }
    }
}
