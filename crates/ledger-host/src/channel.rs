//! The HTTP client for the books server's channel (plan 3.9, D-62): a
//! Unix socket on Unix, a named pipe on Windows. The server speaks HTTP
//! on it; only how a client reaches it differs.

use ledger_mcp::ipc::Endpoint;

/// A client builder that connects to `endpoint` whatever URL it is given
/// (`http://localhost/...`). On Unix the endpoint must outlive the client:
/// a long socket path is reached through a handle it keeps open.
pub fn http_builder(endpoint: &Endpoint) -> reqwest::ClientBuilder {
    #[cfg(unix)]
    {
        reqwest::Client::builder().unix_socket(endpoint.path().to_path_buf())
    }
    #[cfg(windows)]
    {
        reqwest::Client::builder().windows_named_pipe(endpoint.pipe_name().to_string())
    }
}
