//! `wanderbooks update` and `wanderbooks rollback` (plan P11a step 2, 3.9,
//! D-24, D-26), against a local release folder (a `file://` feed) and a
//! key the test makes. Oracles: the INVARIANTS of ledger-host's update.rs
//! and host.rs; the P11a tests update_is_atomic, failed_update_rolls_back,
//! unsigned_update_refused, bridge_survives_update,
//! update_waits_for_a_running_call, package_managed_install_refuses_update,
//! and the rollback laws (kept version, the format gate).
#![cfg(target_os = "linux")]

mod common;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const KNOBS: &str =
    "close_after=2500,backoff=50,exe_poll=100,init_wait=5000,queue_wait=30000,backup_quiet=300";

/// The platform key a manifest names, as the program computes it.
fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn sha256(bytes: &[u8]) -> String {
    ledger_store::journal::sha256_hex(bytes)
}

fn wait_for(secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

/// The program the tests run, stripped once: a debug build is hundreds of
/// megabytes, and every home holds a copy that is hashed and signed.
fn base_program() -> &'static Path {
    static BASE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        // Named for the build, so a run after another reuses it (the
        // `sweep-test-tmp` recipe removes the `wb-` folders after a day).
        let source = Path::new(env!("CARGO_BIN_EXE_wanderbooks"));
        let meta = std::fs::metadata(source).unwrap();
        let stamp = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let dir = std::env::temp_dir().join(format!("wb-update-base-{}-{stamp}", meta.len()));
        let base = dir.join("wanderbooks");
        if !base.is_file() {
            let work = std::env::temp_dir().join(format!("wb-update-work-{}", std::process::id()));
            std::fs::create_dir_all(&work).unwrap();
            let part = work.join("wanderbooks");
            std::fs::copy(source, &part).unwrap();
            // No strip tool: the program as it is.
            let _ = Command::new("strip").arg("--strip-all").arg(&part).status();
            let _ = std::fs::create_dir_all(&dir);
            std::fs::rename(&part, &base).unwrap();
            let _ = std::fs::remove_dir_all(&work);
        }
        base
    })
}

/// A home with a data root, and a copy of the program in `bin/` that the
/// tests update.
struct Home {
    dir: tempfile::TempDir,
    exe: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-update-{tag}-"))
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let exe = bin.join("wanderbooks");
        std::fs::copy(base_program(), &exe).unwrap();
        let home = Self { dir, exe };
        home.runnable();
        home
    }

    /// Wait until the copy can be run (a child another test thread forked
    /// while it was written may hold it open for a moment).
    fn runnable(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match Command::new(&self.exe)
                .arg("--help")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    assert!(Instant::now() < deadline, "the copy stayed busy");
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("the copy can't be run: {e}"),
                Ok(_) => return,
            }
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join(".local/share/wanderbooks")
    }

    fn updates(&self) -> PathBuf {
        self.root().join("updates")
    }

    fn cmd(&self, mode: &str) -> Command {
        let mut c = Command::new(&self.exe);
        c.arg(mode)
            .arg("--root")
            .arg(self.root())
            .args(["--test-knobs", KNOBS])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path());
        c
    }

    fn program(&self) -> Vec<u8> {
        std::fs::read(&self.exe).unwrap()
    }

    fn create(&self, id: &str) {
        let out = self
            .cmd("books")
            .args(["create", id])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let out = self
            .cmd("token")
            .args(["create", "--book", id])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }

    /// `wanderbooks update` (or `rollback`) against `release`.
    fn run(&self, mode: &str, release: Option<&Release>) -> std::process::Output {
        let mut c = Command::new(&self.exe);
        c.arg(mode).arg("--root").arg(self.root());
        if let Some(r) = release {
            c.arg("--feed")
                .arg(format!(
                    "file://{}",
                    r.dir.path().join("manifest.json").display()
                ))
                .arg("--trusted-key")
                .arg(r.dir.path().join("release.pub"));
        }
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path())
            .stdin(Stdio::null());
        c.output().unwrap()
    }

    fn pids(&self, mode: &str) -> Vec<u32> {
        let root = self.root().display().to_string();
        let mut out = Vec::new();
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let args: Vec<String> = raw
                .split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            if args.get(1).map(String::as_str) == Some(mode) && args.contains(&root) {
                out.push(pid);
            }
        }
        out
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self
            .cmd("close")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut found = false;
            for mode in ["serve", "mcp", "host", "server"] {
                for pid in self.pids(mode) {
                    found = true;
                    // SAFETY: a process of this test's own root.
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
            if !found || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// A release folder: a manifest, the program file, and the key it was
/// signed with (its public half in `release.pub`).
struct Release {
    dir: tempfile::TempDir,
    keys: minisign::KeyPair,
}

impl Release {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("wb-release-")
            .tempdir()
            .unwrap();
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        std::fs::write(
            dir.path().join("release.pub"),
            keys.pk.to_box().unwrap().to_string(),
        )
        .unwrap();
        Self { dir, keys }
    }

    /// The signature of `bytes` by `keys`, as the `.sig` file's text.
    fn sign_with(keys: &minisign::KeyPair, bytes: &[u8]) -> String {
        minisign::sign(
            Some(&keys.pk),
            &keys.sk,
            std::io::Cursor::new(bytes),
            Some("test release"),
            Some("signature from a test key"),
        )
        .unwrap()
        .to_string()
    }

    fn b64(text: &str) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// Publish `version` with `program` as this platform's file, signed
    /// by our key; `tweak` changes the manifest's entry before it's
    /// written.
    fn publish(&self, version: &str, program: &[u8], tweak: impl FnOnce(&mut Value)) {
        let name = format!("wanderbooks-{version}");
        std::fs::write(self.dir.path().join(&name), program).unwrap();
        let mut entry = json!({
            "url": name,
            "sha256": sha256(program),
            "signature": Self::b64(&Self::sign_with(&self.keys, program)),
        });
        tweak(&mut entry);
        let manifest = json!({
            "version": version,
            "notes": "a test release",
            "platforms": { platform(): entry },
        });
        std::fs::write(self.dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    }
}

/// The program as a newer release: the same program with a tail (an ELF
/// file runs the same with bytes after its end), so its hash differs.
fn newer(home: &Home, tag: &str) -> Vec<u8> {
    let mut bytes = home.program();
    bytes.extend_from_slice(format!("\nrelease {tag}\n").as_bytes());
    bytes
}

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// What is in `dir`, for "nothing was left behind".
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn an_update_replaces_the_program_keeps_the_old_one_and_rollback_brings_it_back() {
    let home = Home::new("basic");
    let old = home.program();
    let release = Release::new();
    let next = newer(&home, "9.0.0");
    release.publish("9.0.0", &next, |_| {});

    // The newest release is this one; an older offer is refused.
    release.publish("0.1.0", &old, |_| {});
    let same = home.run("update", Some(&release));
    assert!(
        same.status.success() && text(&same).contains("newest"),
        "{}",
        text(&same)
    );
    release.publish("0.0.1", &old, |_| {});
    let older = home.run("update", Some(&release));
    assert!(!older.status.success(), "{}", text(&older));
    assert_eq!(home.program(), old);
    assert!(!home.updates().join("previous").exists());
    release.publish("9.0.0", &next, |_| {});

    let look = home.run("update", None);
    assert!(!look.status.success(), "no key, no feed: {}", text(&look));
    assert!(text(&look).contains("no release key"), "{}", text(&look));

    let out = home.run("update", Some(&release));
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("from 0.1.0 to 9.0.0"), "{}", text(&out));
    assert_eq!(home.program(), next, "the new program is in place");
    let kept = home.updates().join("previous");
    assert_eq!(std::fs::read(kept.join("wanderbooks")).unwrap(), old);
    let meta: Value =
        serde_json::from_slice(&std::fs::read(kept.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["version"], "0.1.0");
    assert!(meta["format"].as_u64().unwrap() >= 2);
    // Nothing staged is left, and the program's folder holds the program
    // alone.
    assert_eq!(names(&home.updates()), ["previous", "update.lock"]);
    assert_eq!(names(home.exe.parent().unwrap()), ["wanderbooks"]);

    // Rollback: the old program is back, and nothing is kept to go back to.
    let out = home.run("rollback", None);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(home.program(), old);
    assert!(!kept.exists());
    let nothing = home.run("rollback", None);
    assert!(!nothing.status.success());
    assert!(
        text(&nothing).contains("no earlier version"),
        "{}",
        text(&nothing)
    );
}

#[test]
fn update_is_atomic() {
    let home = Home::new("atomic");
    let old = home.program();
    let release = Release::new();
    let next = newer(&home, "atomic");
    release.publish("9.0.0", &next, |_| {});
    // A reader of the program file sees the old one or the new, whole,
    // all through the swap.
    let (old_hash, new_hash) = (sha256(&old), sha256(&next));
    let path = home.exe.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut reads = 0u32;
            let mut bad = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let h = sha256(&bytes);
                        if h != old_hash && h != new_hash {
                            bad.push(format!("{} bytes", bytes.len()));
                        }
                        reads += 1;
                    }
                    Err(e) => bad.push(e.to_string()),
                }
            }
            (reads, bad)
        })
    };
    let out = home.run("update", Some(&release));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (reads, bad) = seen.join().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(reads > 0);
    assert!(bad.is_empty(), "a reader saw a part of a program: {bad:?}");
    assert_eq!(home.program(), next);
}

#[test]
fn failed_update_rolls_back() {
    let home = Home::new("failed");
    let old = home.program();
    let release = Release::new();
    // Validly signed, but not a program that starts.
    release.publish("9.0.0", b"#!/bin/sh\nexit 3\n", |_| {});
    let out = home.run("update", Some(&release));
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("didn't start"), "{}", text(&out));
    assert_eq!(home.program(), old, "the old program is back");
    assert_eq!(names(home.exe.parent().unwrap()), ["wanderbooks"]);
    // And it still works.
    home.runnable();
    // The failed one is not left staged to be tried again.
    assert!(!home.updates().join("9.0.0").exists());
}

#[test]
fn unsigned_update_refused() {
    let home = Home::new("unsigned");
    let old = home.program();
    let release = Release::new();
    let next = newer(&home, "unsigned");
    let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    type Tweak = Box<dyn FnOnce(&mut Value)>;
    let mut cases: Vec<(&str, Tweak)> = Vec::new();
    // A bad signature: a good one by another key.
    let by_other = Release::b64(&Release::sign_with(&other, &next));
    cases.push((
        "another key",
        Box::new(move |e| e["signature"] = json!(by_other)),
    ));
    // A signature of other bytes.
    let of_other_bytes = Release::b64(&Release::sign_with(&release.keys, b"something else"));
    cases.push((
        "other bytes",
        Box::new(move |e| e["signature"] = json!(of_other_bytes)),
    ));
    // A sha that differs from the manifest's.
    cases.push(("sha256", Box::new(|e| e["sha256"] = json!("0".repeat(64)))));
    cases.push(("no signature", Box::new(|e| e["signature"] = json!(""))));
    cases.push((
        "not a signature",
        Box::new(|e| e["signature"] = json!("AAAA")),
    ));
    for (name, tweak) in cases {
        release.publish("9.0.0", &next, tweak);
        let out = home.run("update", Some(&release));
        assert!(!out.status.success(), "{name}: {}", text(&out));
        assert!(text(&out).contains("refused"), "{name}: {}", text(&out));
        assert_eq!(home.program(), old, "{name}: the program changed");
        assert!(!home.updates().join("previous").exists(), "{name}: kept");
        assert!(!home.updates().join("9.0.0").exists(), "{name}: staged");
        assert_eq!(names(home.exe.parent().unwrap()), ["wanderbooks"], "{name}");
    }
    // The key must be named only with a local feed.
    let out = Command::new(&home.exe)
        .args(["update", "--root"])
        .arg(home.root())
        .args([
            "--feed",
            "https://example.invalid/manifest.json",
            "--trusted-key",
        ])
        .arg(release.dir.path().join("release.pub"))
        .env_clear()
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("file:// feed"), "{}", text(&out));
}

#[test]
fn package_managed_install_refuses_update() {
    let home = Home::new("package");
    let old = home.program();
    let share = home.dir.path().join("share/wanderbooks");
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(share.join("install-method"), "deb\n").unwrap();
    let release = Release::new();
    release.publish("9.0.0", &newer(&home, "pkg"), |_| {});
    for mode in ["update", "rollback"] {
        let out = home.run(mode, Some(&release));
        let out = if mode == "rollback" {
            home.run(mode, None)
        } else {
            out
        };
        assert!(!out.status.success(), "{mode}: {}", text(&out));
        assert!(
            text(&out).contains("managed by deb"),
            "{mode}: {}",
            text(&out)
        );
        assert!(
            text(&out).contains("package manager"),
            "{mode}: {}",
            text(&out)
        );
    }
    assert_eq!(home.program(), old);
    assert!(!home.updates().exists(), "nothing was made");
}

#[test]
fn rollback_refuses_after_a_book_consented_to_a_newer_format() {
    let home = Home::new("consent");
    home.create("acme");
    let release = Release::new();
    let next = newer(&home, "consent");
    release.publish("9.0.0", &next, |_| {});
    let out = home.run("update", Some(&release));
    assert!(out.status.success(), "{}", text(&out));
    // The version kept understood less than this one writes into new
    // books (a feature it lacks).
    let meta_path = home.updates().join("previous/meta.json");
    let mut meta: Value = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    let features = meta["features"].as_array_mut().unwrap();
    assert!(!features.is_empty());
    features.pop();
    std::fs::write(&meta_path, meta.to_string()).unwrap();
    let out = home.run("rollback", None);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("newer journal format"),
        "{}",
        text(&out)
    );
    assert_eq!(home.program(), next, "the program stayed");
    assert!(home.updates().join("previous/wanderbooks").is_file());
    // Declared what the kept version knows: it goes back.
    let mut meta: Value = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    meta["features"] = json!(ledger_store::format::current().features);
    std::fs::write(&meta_path, meta.to_string()).unwrap();
    let out = home.run("rollback", None);
    assert!(out.status.success(), "{}", text(&out));
}

/// The bridge over stdio, spoken to as an MCP client.
struct Agent {
    child: std::process::Child,
    lines: mpsc::Receiver<String>,
    next: u64,
}

impl Agent {
    fn start(home: &Home, book: &str) -> Self {
        let mut c = home.cmd("mcp");
        c.args(["--book", book]);
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (tx, lines) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut a = Self {
            child,
            lines,
            next: 1,
        };
        a.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        );
        a.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        a
    }

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no answer to {method}"));
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let r = common::tools_call(|m, p| self.request(m, p), tool, args);
        if let Some(e) = r.get("error") {
            return Err(e["message"].as_str().unwrap_or_default().to_string());
        }
        if r["result"]["isError"] == true {
            return Err(r["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .into());
        }
        Ok(r["result"]["structuredContent"].clone())
    }

    fn add_account(&mut self, code: &str) -> Result<Value, String> {
        self.call(
            "books.add_account",
            json!({"code": code, "name": format!("A{code}"), "account_type": "expense"}),
        )
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Update while an agent's session writes without pause: every write is
/// answered and made once, the session carries on with no reconnect, the
/// host and server run the new file afterwards, and the head is the one
/// the writes made.
fn update_under_writes(tag: &str) -> (Home, Agent, Vec<String>) {
    let home = Home::new(tag);
    home.create("acme");
    let release = Release::new();
    let next = newer(&home, tag);
    release.publish("9.0.0", &next, |_| {});
    let mut agent = Agent::start(&home, "acme");
    agent.add_account("4000").unwrap();
    let before = agent.request("tools/list", json!({}));
    let host_before = wait_host(&home);

    let updating = {
        let exe = home.exe.clone();
        let (root, dir) = (home.root(), home.dir.path().to_path_buf());
        let feed = format!(
            "file://{}",
            release.dir.path().join("manifest.json").display()
        );
        let key = release.dir.path().join("release.pub");
        std::thread::spawn(move || {
            let _keep = &release;
            Command::new(exe)
                .args(["update", "--root"])
                .arg(root)
                .args(["--feed", &feed, "--trusted-key"])
                .arg(key)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", dir)
                .output()
                .unwrap()
        })
    };
    let mut written = vec!["4000".to_string()];
    let mut k = 0u32;
    // Writes all through the update and the restart it causes.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut restarted = false;
    while !restarted {
        assert!(
            Instant::now() < deadline,
            "the host never restarted onto the update"
        );
        let code = format!("{}", 5000 + k);
        k += 1;
        let r = agent.add_account(&code);
        assert!(r.is_ok(), "a write across the update: {r:?}");
        written.push(code);
        if updating.is_finished() {
            restarted = wait_for(1, || {
                home.pids("host").len() == 1 && home.pids("host")[0] != host_before
            });
        }
    }
    let out = updating.join().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(home.program(), next);
    // The host and the server run the program now on disk.
    let host = wait_host(&home);
    assert_ne!(host, host_before);
    let exe = |pid: u32| std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
    assert!(wait_for(20, || !home.pids("server").is_empty()));
    assert_eq!(exe(home.pids("server")[0]), exe(host));
    assert!(!exe(host).to_string_lossy().contains("(deleted)"));
    // The session carries on, its tools unchanged.
    assert!(agent.add_account("4999").is_ok());
    written.push("4999".into());
    let after = agent.request("tools/list", json!({}));
    assert_eq!(before["result"], after["result"]);
    (home, agent, written)
}

fn wait_host(home: &Home) -> u32 {
    assert!(
        wait_for(20, || home.pids("host").len() == 1),
        "no single host"
    );
    home.pids("host")[0]
}

#[test]
fn bridge_survives_update() {
    let (_home, mut agent, written) = update_under_writes("bridge");
    let chart = agent.call("books.chart", json!({})).unwrap();
    let in_books = chart["accounts"].as_array().unwrap().len();
    assert!(in_books >= written.len(), "{in_books} < {}", written.len());
}

#[test]
fn update_waits_for_a_running_call() {
    // No call is cut off or made twice: each account written across the
    // update is in the books exactly once.
    let (_home, mut agent, written) = update_under_writes("running");
    let chart = agent.call("books.chart", json!({})).unwrap();
    let mut in_books: Vec<String> = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["code"].as_str().map(str::to_string))
        .filter(|c| c.starts_with('4') || c.starts_with('5'))
        .collect();
    let mut written = written;
    in_books.sort();
    written.sort();
    assert_eq!(
        in_books, written,
        "a write lost, or made twice, across the update"
    );
}
