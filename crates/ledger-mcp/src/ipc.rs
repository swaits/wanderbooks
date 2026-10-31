//! The local transport, one seam for both families of system (plan 3.9,
//! D-62): Unix sockets in the root's `run/` on Unix, named pipes on
//! Windows, each reachable by this account alone.
//!
//! ```text
//! GOAL: the books server, the host and everything that talks to them
//!       meet on a channel that only this account can open, and each end
//!       checks that the other end is this account.
//! SOURCE: plan/served.md 3.9 ("the local transport, the peer check":
//!         0600 sockets plus the peer's uid; on Windows a pipe with an
//!         owner-only DACL plus the peer process's SID), D-32 (nothing
//!         here confines a process), D-62.
//! INVARIANTS:
//!   1. A listener is made for this account alone: Unix sockets 0600; a
//!      Windows pipe with a DACL of one allow entry (this account's SID)
//!      and remote clients refused.
//!   2. [`Peer::peer_is_us`] is true only when the system says the other
//!      end is this account (the effective uid; the user SID of the
//!      process at the other end of the pipe); any doubt is false.
//!   3. A Windows pipe's name carries a hash of the root's `run/` folder,
//!      so two roots on one machine never share a pipe, and the first
//!      instance can be made only once (a second server is refused).
//!   4. Windows keeps idle pipe instances waiting, so a client never
//!      finds every instance busy while the server makes the next one.
//!   5. A read with a time limit gives up at the limit with `TimedOut`
//!      (Unix: `WouldBlock`); a pipe whose other end has gone reads as
//!      the end of the stream.
//! ```

use std::io;

pub use sys::*;

/// Who is at the other end of a connection.
pub trait Peer {
    /// Whether the other end is this account (INVARIANT 2).
    fn peer_is_us(&self) -> bool;
    /// The other end's process, where the system says it.
    fn peer_pid(&self) -> Option<u32>;
}

/// Whether the other end of `stream` is this account.
pub fn peer_is_us(stream: &impl Peer) -> bool {
    stream.peer_is_us()
}

/// The other end's process, where the system says.
pub fn peer_pid(stream: &impl Peer) -> Option<u32> {
    stream.peer_pid()
}

/// One of the signals that ask a program to stop: on Unix SIGTERM,
/// SIGINT and SIGHUP; on Windows Ctrl+C, Ctrl+Break, closing the console
/// and shutdown.
pub struct StopSignal(sys::RawStop);

impl StopSignal {
    /// The next time it arrives; `None` once it can't arrive again.
    pub async fn recv(&mut self) -> Option<()> {
        self.0.recv().await
    }
}

/// Watch for every signal that asks a program to stop.
pub fn stop_signals() -> io::Result<Vec<StopSignal>> {
    sys::raw_stops().map(|all| all.into_iter().map(StopSignal).collect())
}

#[cfg(unix)]
mod sys {
    use super::Peer;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};

    /// A connection a listener accepted.
    pub type Accepted = tokio::net::UnixStream;
    /// A connection this side made.
    pub type Connected = tokio::net::UnixStream;
    /// A connection this side made, for blocking code.
    pub type StdStream = std::os::unix::net::UnixStream;
    /// What [`Listener::addr_of`] says of a connection.
    pub type Addr = tokio::net::unix::SocketAddr;
    /// The reading half of an accepted connection.
    pub type AcceptedRead = tokio::net::unix::OwnedReadHalf;
    /// The writing half of an accepted connection.
    pub type AcceptedWrite = tokio::net::unix::OwnedWriteHalf;
    /// The reading half of a connection this side made.
    pub type ConnectedRead = tokio::net::unix::OwnedReadHalf;
    /// The writing half of a connection this side made.
    pub type ConnectedWrite = tokio::net::unix::OwnedWriteHalf;

    /// Split an accepted connection into its halves.
    pub fn split_accepted(stream: Accepted) -> (AcceptedRead, AcceptedWrite) {
        stream.into_split()
    }

    /// Split a connection this side made into its halves.
    pub fn split_connected(stream: Connected) -> (ConnectedRead, ConnectedWrite) {
        stream.into_split()
    }

    /// Where a channel is: a socket `name` in the root's `run/`.
    #[derive(Debug)]
    pub struct Endpoint {
        full: PathBuf,
        short: ledger_store::root::SocketPath,
    }

    impl Endpoint {
        /// The channel `name` in the root's `run/` folder `dir`.
        pub fn new(dir: &Path, name: &str) -> io::Result<Endpoint> {
            Ok(Endpoint {
                full: dir.join(name),
                short: ledger_store::root::SocketPath::new(dir, name)?,
            })
        }

        /// The path to bind or connect to (good while this lives).
        pub fn path(&self) -> &Path {
            self.short.path()
        }
    }

    /// A listener that is this account's alone (INVARIANT 1).
    pub struct Listener(tokio::net::UnixListener);

    impl Listener {
        /// Bind `endpoint`, replacing a stale socket (only a server that
        /// holds its lock gets here).
        pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
            match std::fs::remove_file(&endpoint.full) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let listener = tokio::net::UnixListener::bind(endpoint.path())?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&endpoint.full, std::fs::Permissions::from_mode(0o600))?;
            Ok(Listener(listener))
        }

        /// The next connection, whoever it is from.
        pub async fn accept(&mut self) -> io::Result<Accepted> {
            self.0.accept().await.map(|(stream, _)| stream)
        }

        /// The address of an accepted connection.
        pub fn addr_of(stream: &Accepted) -> io::Result<Addr> {
            stream.peer_addr()
        }

        /// The address this listener is bound to.
        pub fn local_addr(&self) -> io::Result<Addr> {
            self.0.local_addr()
        }
    }

    /// Connect to `endpoint`.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<Connected> {
        tokio::net::UnixStream::connect(endpoint.path()).await
    }

    /// Connect to `endpoint`, for blocking code.
    pub fn connect_std(endpoint: &Endpoint) -> io::Result<StdStream> {
        std::os::unix::net::UnixStream::connect(endpoint.path())
    }

    /// The user ID at the other end of a Unix socket.
    pub fn peer_uid(fd: &impl AsRawFd) -> io::Result<u32> {
        let fd = fd.as_raw_fd();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let mut cred = libc::ucred {
                pid: 0,
                uid: 0,
                gid: 0,
            };
            let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: `cred` and `len` are valid for writes of the sizes given.
            let rc = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut len,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            if len as usize != std::mem::size_of::<libc::ucred>() {
                return Err(io::Error::other("the peer's credentials came back short"));
            }
            Ok(cred.uid)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let mut uid: libc::uid_t = 0;
            let mut gid: libc::gid_t = 0;
            // SAFETY: `uid` and `gid` are valid for writes.
            if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(uid)
        }
    }

    /// This process's user ID.
    pub fn own_uid() -> u32 {
        // SAFETY: geteuid has no preconditions and can't fail.
        unsafe { libc::geteuid() }
    }

    fn pid_of(fd: &impl AsRawFd) -> Option<u32> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let mut cred = libc::ucred {
                pid: 0,
                uid: 0,
                gid: 0,
            };
            let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: `cred` and `len` are valid for writes of the sizes given.
            let rc = unsafe {
                libc::getsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut len,
                )
            };
            (rc == 0 && len as usize == std::mem::size_of::<libc::ucred>() && cred.pid > 0)
                .then_some(cred.pid as u32)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = fd;
            None
        }
    }

    impl<T: AsRawFd> Peer for T {
        fn peer_is_us(&self) -> bool {
            peer_uid(self).is_ok_and(|uid| uid == own_uid())
        }

        fn peer_pid(&self) -> Option<u32> {
            pid_of(self)
        }
    }

    /// One stop signal.
    pub struct RawStop(tokio::signal::unix::Signal);

    impl RawStop {
        pub(super) async fn recv(&mut self) -> Option<()> {
            self.0.recv().await
        }
    }

    pub(super) fn raw_stops() -> io::Result<Vec<RawStop>> {
        use tokio::signal::unix::{SignalKind, signal};
        [
            SignalKind::terminate(),
            SignalKind::interrupt(),
            SignalKind::hangup(),
        ]
        .into_iter()
        .map(|kind| signal(kind).map(RawStop))
        .collect()
    }
}

#[cfg(windows)]
mod sys {
    use super::Peer;
    use ledger_store::winsec;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use tokio::sync::mpsc;

    /// ERROR_PIPE_BUSY: every instance of the pipe is in use for now.
    const PIPE_BUSY: i32 = 231;
    /// ERROR_BROKEN_PIPE: the other end has gone.
    const BROKEN_PIPE: i32 = 109;
    /// SECURITY_IDENTIFICATION: the server may learn who the client is
    /// and may not act as the client.
    const IDENTIFICATION: u32 = 0x0001_0000;
    /// How long a client waits for a busy pipe (INVARIANT 4).
    const BUSY_WAIT: Duration = Duration::from_secs(3);
    /// Instances kept waiting for a client (INVARIANT 4).
    const IDLE_INSTANCES: usize = 16;

    /// A connection a listener accepted.
    pub type Accepted = NamedPipeServer;
    /// A connection this side made.
    pub type Connected = NamedPipeClient;
    /// What [`Listener::addr_of`] says of a connection: nothing.
    pub type Addr = ();
    /// The reading half of an accepted connection.
    pub type AcceptedRead = tokio::io::ReadHalf<NamedPipeServer>;
    /// The writing half of an accepted connection.
    pub type AcceptedWrite = tokio::io::WriteHalf<NamedPipeServer>;
    /// The reading half of a connection this side made.
    pub type ConnectedRead = tokio::io::ReadHalf<NamedPipeClient>;
    /// The writing half of a connection this side made.
    pub type ConnectedWrite = tokio::io::WriteHalf<NamedPipeClient>;

    /// Split an accepted connection into its halves.
    pub fn split_accepted(stream: Accepted) -> (AcceptedRead, AcceptedWrite) {
        tokio::io::split(stream)
    }

    /// Split a connection this side made into its halves.
    pub fn split_connected(stream: Connected) -> (ConnectedRead, ConnectedWrite) {
        tokio::io::split(stream)
    }

    /// Where a channel is: the pipe `\\.\pipe\wanderbooks-<hash of the
    /// root's run/ folder>-<name>` (INVARIANT 3).
    #[derive(Debug, Clone)]
    pub struct Endpoint {
        name: String,
    }

    impl Endpoint {
        /// The channel `name` for the root's `run/` folder `dir`.
        pub fn new(dir: &Path, name: &str) -> io::Result<Endpoint> {
            use sha2::{Digest, Sha256};
            // The folder as the system names it (a short 8.3 name and its
            // long one are one folder; every process must hash the same
            // text), without the `\\?\` prefix canonicalizing adds; a
            // folder name is not case-sensitive here.
            let dir = std::fs::canonicalize(dir).or_else(|_| std::path::absolute(dir))?;
            let text = dir.to_string_lossy().to_lowercase();
            let key = text.strip_prefix(r"\\?\").unwrap_or(&text);
            let digest = Sha256::digest(key.as_bytes());
            let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
            let name = name.strip_suffix(".sock").unwrap_or(name);
            Ok(Endpoint {
                name: format!(r"\\.\pipe\wanderbooks-{hash}-{name}"),
            })
        }

        /// The pipe's name.
        pub fn pipe_name(&self) -> &str {
            &self.name
        }
    }

    fn make(name: &str, security: &winsec::OwnerOnly, first: bool) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        // SAFETY: the pointer is to security attributes that `security`
        // keeps alive for the call, which copies them.
        unsafe { options.create_with_security_attributes_raw(name, security.as_raw()) }
    }

    /// A listener that is this account's alone (INVARIANT 1): a pool of
    /// idle instances (INVARIANT 4), each handing its connection on when
    /// a client comes.
    pub struct Listener {
        accepted: mpsc::UnboundedReceiver<io::Result<NamedPipeServer>>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }

    impl Listener {
        /// Make the pipe. Refused when another server already has it
        /// (INVARIANT 3).
        pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
            let security = Arc::new(winsec::OwnerOnly::new()?);
            let first = make(&endpoint.name, &security, true)?;
            let (tx, accepted) = mpsc::unbounded_channel();
            let mut tasks = Vec::new();
            let mut instance = Some(first);
            for _ in 0..IDLE_INSTANCES {
                let server = match instance.take() {
                    Some(server) => server,
                    None => make(&endpoint.name, &security, false)?,
                };
                tasks.push(tokio::spawn(wait_for_clients(
                    server,
                    endpoint.name.clone(),
                    security.clone(),
                    tx.clone(),
                )));
            }
            Ok(Listener { accepted, tasks })
        }

        /// The next connection, whoever it is from.
        pub async fn accept(&mut self) -> io::Result<Accepted> {
            match self.accepted.recv().await {
                Some(result) => result,
                None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            }
        }

        /// The address of an accepted connection: none.
        pub fn addr_of(_stream: &Accepted) -> io::Result<Addr> {
            Ok(())
        }

        /// The address this listener is bound to: none.
        pub fn local_addr(&self) -> io::Result<Addr> {
            Ok(())
        }
    }

    /// One idle instance: wait for a client, put a fresh instance in its
    /// place, and hand the connected one on.
    async fn wait_for_clients(
        mut server: NamedPipeServer,
        name: String,
        security: Arc<winsec::OwnerOnly>,
        tx: mpsc::UnboundedSender<io::Result<NamedPipeServer>>,
    ) {
        loop {
            if let Err(e) = server.connect().await {
                // This instance is no good: say so, and put a fresh one in
                // its place (nothing connected is handed on).
                if tx.send(Err(e)).is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Ok(fresh) = make(&name, &security, false) {
                    server = fresh;
                }
                continue;
            }
            let next = loop {
                match make(&name, &security, false) {
                    Ok(next) => break next,
                    Err(e) => {
                        if tx.send(Err(e)).is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            };
            let connected = std::mem::replace(&mut server, next);
            if tx.send(Ok(connected)).is_err() {
                return;
            }
        }
    }

    /// Connect to `endpoint`, waiting a little while every instance is
    /// busy (INVARIANT 4).
    pub async fn connect(endpoint: &Endpoint) -> io::Result<Connected> {
        let deadline = Instant::now() + BUSY_WAIT;
        loop {
            match ClientOptions::new()
                .security_qos_flags(IDENTIFICATION)
                .open(&endpoint.name)
            {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// A connection this side made, for blocking code.
    #[derive(Debug)]
    pub struct StdStream {
        file: std::fs::File,
        read_timeout: Mutex<Option<Duration>>,
    }

    impl StdStream {
        /// Give up reading after `timeout` (INVARIANT 5); `None` waits
        /// for ever.
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            *self.read_timeout.lock().unwrap_or_else(|e| e.into_inner()) = timeout;
            Ok(())
        }

        fn read_some(&self, buf: &mut [u8]) -> io::Result<usize> {
            let timeout = *self.read_timeout.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(timeout) = timeout {
                // A blocking read can't be cut short, so it is made only
                // when something is waiting.
                let deadline = Instant::now() + timeout;
                loop {
                    match winsec::pipe_bytes_waiting(&self.file) {
                        Ok(0) => {}
                        Ok(_) => break,
                        Err(e) if e.raw_os_error() == Some(BROKEN_PIPE) => return Ok(0),
                        Err(e) => return Err(e),
                    }
                    if Instant::now() >= deadline {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            match (&self.file).read(buf) {
                Err(e) if e.raw_os_error() == Some(BROKEN_PIPE) => Ok(0),
                other => other,
            }
        }
    }

    use std::io::{Read, Write};

    impl Read for StdStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read_some(buf)
        }
    }

    impl Read for &StdStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read_some(buf)
        }
    }

    impl Write for StdStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.file).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.file).flush()
        }
    }

    impl Write for &StdStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.file).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.file).flush()
        }
    }

    /// Connect to `endpoint`, for blocking code, waiting a little while
    /// every instance is busy (INVARIANT 4).
    pub fn connect_std(endpoint: &Endpoint) -> io::Result<StdStream> {
        let deadline = Instant::now() + BUSY_WAIT;
        loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(IDENTIFICATION)
                .open(&endpoint.name)
            {
                Ok(file) => {
                    return Ok(StdStream {
                        file,
                        read_timeout: Mutex::new(None),
                    });
                }
                Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }
    }

    impl Peer for NamedPipeServer {
        fn peer_is_us(&self) -> bool {
            self.peer_pid().is_some_and(winsec::pid_is_us)
        }

        fn peer_pid(&self) -> Option<u32> {
            winsec::pipe_client_pid(self).ok()
        }
    }

    impl Peer for NamedPipeClient {
        fn peer_is_us(&self) -> bool {
            self.peer_pid().is_some_and(winsec::pid_is_us)
        }

        fn peer_pid(&self) -> Option<u32> {
            winsec::pipe_server_pid(self).ok()
        }
    }

    impl Peer for StdStream {
        fn peer_is_us(&self) -> bool {
            self.peer_pid().is_some_and(winsec::pid_is_us)
        }

        fn peer_pid(&self) -> Option<u32> {
            winsec::pipe_server_pid(&self.file).ok()
        }
    }

    /// One stop signal.
    pub enum RawStop {
        /// Ctrl+C.
        CtrlC(tokio::signal::windows::CtrlC),
        /// Ctrl+Break.
        CtrlBreak(tokio::signal::windows::CtrlBreak),
        /// The console window is closed.
        CtrlClose(tokio::signal::windows::CtrlClose),
        /// The system is shutting down.
        CtrlShutdown(tokio::signal::windows::CtrlShutdown),
    }

    impl RawStop {
        pub(super) async fn recv(&mut self) -> Option<()> {
            match self {
                RawStop::CtrlC(s) => s.recv().await,
                RawStop::CtrlBreak(s) => s.recv().await,
                RawStop::CtrlClose(s) => s.recv().await,
                RawStop::CtrlShutdown(s) => s.recv().await,
            }
        }
    }

    pub(super) fn raw_stops() -> io::Result<Vec<RawStop>> {
        use tokio::signal::windows;
        Ok(vec![
            RawStop::CtrlC(windows::ctrl_c()?),
            RawStop::CtrlBreak(windows::ctrl_break()?),
            RawStop::CtrlClose(windows::ctrl_close()?),
            RawStop::CtrlShutdown(windows::ctrl_shutdown()?),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    fn endpoint(dir: &tempfile::TempDir) -> Endpoint {
        Endpoint::new(dir.path(), "test.sock").unwrap()
    }

    /// Both ends of a connection are this account, and name this process.
    #[tokio::test]
    async fn a_listener_and_a_client_talk_and_each_knows_the_other_is_us() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(&dir);
        let mut listener = Listener::bind(&ep).unwrap();
        let client = connect(&ep).await.unwrap();
        let server = listener.accept().await.unwrap();
        assert!(peer_is_us(&server));
        assert!(peer_is_us(&client));
        #[cfg(any(windows, target_os = "linux"))]
        {
            assert_eq!(peer_pid(&server), Some(std::process::id()));
            assert_eq!(peer_pid(&client), Some(std::process::id()));
        }
        let (read, mut write) = split_accepted(server);
        let (cread, mut cwrite) = split_connected(client);
        cwrite.write_all(b"hello\n").await.unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(read)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "hello\n");
        write.write_all(b"back\n").await.unwrap();
        let mut line = String::new();
        tokio::io::BufReader::new(cread)
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "back\n");
    }

    /// A blocking client gives up at its limit, and reads the end of the
    /// stream once the server has gone (INVARIANT 5).
    #[tokio::test]
    async fn a_blocking_client_times_out_and_reads_the_end_when_the_server_goes() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(&dir);
        let mut listener = Listener::bind(&ep).unwrap();
        let ep2 = ep_for(&dir);
        let client = tokio::task::spawn_blocking(move || connect_std(&ep2).unwrap())
            .await
            .unwrap();
        let server = listener.accept().await.unwrap();
        assert!(peer_is_us(&client));
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (client, kind) = tokio::task::spawn_blocking(move || {
            let mut byte = [0u8; 1];
            let kind = (&client).read(&mut byte).unwrap_err().kind();
            (client, kind)
        })
        .await
        .unwrap();
        assert!(
            matches!(
                kind,
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "{kind:?}"
        );
        drop(server);
        let n = tokio::task::spawn_blocking(move || {
            let mut byte = [0u8; 1];
            (&client).read(&mut byte).unwrap()
        })
        .await
        .unwrap();
        assert_eq!(n, 0, "the end of the stream");
    }

    fn ep_for(dir: &tempfile::TempDir) -> Endpoint {
        Endpoint::new(dir.path(), "test.sock").unwrap()
    }

    /// Many clients at once all get in (INVARIANT 4).
    #[tokio::test]
    async fn many_clients_at_once_all_connect() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(&dir);
        let mut listener = Listener::bind(&ep).unwrap();
        let ep = std::sync::Arc::new(ep);
        let mut clients = Vec::new();
        for _ in 0..48 {
            let ep = ep.clone();
            clients.push(tokio::spawn(async move { connect(&ep).await }));
        }
        let mut accepted = Vec::new();
        for _ in 0..48 {
            accepted.push(
                tokio::time::timeout(Duration::from_secs(10), listener.accept())
                    .await
                    .expect("accepted in time")
                    .unwrap(),
            );
        }
        for c in clients {
            c.await.unwrap().unwrap();
        }
    }

    /// Nothing listening: not found, so callers say "not running".
    #[tokio::test]
    async fn nothing_listening_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = connect(&endpoint(&dir)).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ),
            "{err:?}"
        );
    }

    /// Two roots on one machine never share a pipe (INVARIANT 3), and a
    /// second server cannot take the first one's pipe.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_pipe_is_per_root_and_cannot_be_taken_twice() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_ne!(endpoint(&a).pipe_name(), endpoint(&b).pipe_name());
        assert!(
            endpoint(&a)
                .pipe_name()
                .starts_with(r"\\.\pipe\wanderbooks-")
        );
        let _first = Listener::bind(&endpoint(&a)).unwrap();
        assert!(Listener::bind(&endpoint(&a)).is_err());
    }
}
