//! The stdio wire, in front of rmcp: what JSON-RPC 2.0 requires that the
//! transport doesn't do.
//!
//! ```text
//! GOAL: every line the client sends gets the response JSON-RPC 2.0 says
//!       it gets, even one rmcp would drop.
//! SOURCE: docs/spec/sources/jsonrpc-2.0.md (5: one response per request,
//!         id echoed, or null when it can't be detected; 5.1: -32700 Parse
//!         error, -32600 Invalid Request); MCP 2025-06-18 has no batches.
//! INVARIANTS:
//!   1. A line that isn't JSON gets {"code": -32700} with id null.
//!   2. JSON that isn't a request, notification, or response object (an
//!      array, a scalar, an object without "jsonrpc": "2.0" and a string
//!      "method" or a result/error) gets {"code": -32600}, with the
//!      line's id when it has a valid one (string, number, null), else
//!      null.
//!   3. Everything else goes to rmcp unchanged; a blank line is ignored.
//!   4. Output lines are written whole: replies made here never split a
//!      line rmcp writes.
//! ```

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// What to do with one line from the client.
#[derive(Debug, PartialEq)]
pub enum Line {
    /// Hand it to rmcp.
    Forward,
    /// Ignore it (blank).
    Skip,
    /// Answer it here with this JSON-RPC response.
    Reply(Value),
}

/// Classify one line (INVARIANTS 1-3).
pub fn classify(line: &str) -> Line {
    if line.trim().is_empty() {
        return Line::Skip;
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Line::Reply(error(Value::Null, -32700, "Parse error")),
    };
    let Some(obj) = v.as_object() else {
        return Line::Reply(error(Value::Null, -32600, "Invalid Request"));
    };
    let id = match obj.get("id") {
        Some(id @ (Value::String(_) | Value::Number(_) | Value::Null)) => id.clone(),
        _ => Value::Null,
    };
    let version_ok = obj.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    let id_ok = match obj.get("id") {
        None => true,
        Some(v) => matches!(v, Value::String(_) | Value::Number(_) | Value::Null),
    };
    let request = obj.get("method").is_some_and(Value::is_string);
    let response =
        obj.contains_key("id") && (obj.contains_key("result") ^ obj.contains_key("error"));
    if version_ok && id_ok && (request || response) {
        Line::Forward
    } else {
        Line::Reply(error(id, -32600, "Invalid Request"))
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Run the wire between the real `input`/`output` and rmcp's ends:
/// returns (what rmcp reads, what rmcp writes) and the task that must be
/// awaited after rmcp finishes, so the last lines are flushed.
pub fn spawn<R, W>(
    input: R,
    mut output: W,
) -> (
    tokio::io::DuplexStream,
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<()>,
)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    const BUF: usize = 1 << 20;
    let (rmcp_in, mut feed) = tokio::io::duplex(BUF);
    let (rmcp_out, from_rmcp) = tokio::io::duplex(BUF);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let replies = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(input).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match classify(&line) {
                Line::Skip => {}
                Line::Forward => {
                    if feed
                        .write_all(format!("{line}\n").as_bytes())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Line::Reply(v) => {
                    let _ = replies.send(v.to_string());
                }
            }
        }
        // Dropping `feed` ends rmcp's input.
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(from_rmcp).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if output
                .write_all(format!("{line}\n").as_bytes())
                .await
                .is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });
    (rmcp_in, rmcp_out, writer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(l: Line) -> (i64, Value) {
        match l {
            Line::Reply(v) => (v["error"]["code"].as_i64().unwrap(), v["id"].clone()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn classifies_by_the_jsonrpc_spec() {
        assert_eq!(
            code(classify(r#"{"jsonrpc":"2.0","id":99,"method":"#)),
            (-32700, Value::Null)
        );
        assert_eq!(code(classify("not json")), (-32700, Value::Null));
        assert_eq!(code(classify("[]")), (-32600, Value::Null));
        assert_eq!(code(classify("[1]")), (-32600, Value::Null));
        assert_eq!(code(classify("5")), (-32600, Value::Null));
        assert_eq!(
            code(classify(r#"{"jsonrpc":"2.0","id":98}"#)),
            (-32600, json!(98))
        );
        assert_eq!(
            code(classify(r#"{"jsonrpc":"1.0","id":"a","method":"x"}"#)),
            (-32600, json!("a"))
        );
        assert_eq!(
            code(classify(r#"{"jsonrpc":"2.0","id":[1],"method":"x"}"#)),
            (-32600, Value::Null)
        );
        assert_eq!(
            code(classify(r#"{"jsonrpc":"2.0","method":5}"#)),
            (-32600, Value::Null)
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
            Line::Forward
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Line::Forward
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#),
            Line::Forward
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":7,"error":{"code":1,"message":"x"}}"#),
            Line::Forward
        );
        assert_eq!(
            code(classify(
                r#"{"jsonrpc":"2.0","id":7,"result":{},"error":{}}"#
            )),
            (-32600, json!(7))
        );
        assert_eq!(classify("   "), Line::Skip);
    }

    /// Counts the lines written to it; every flush fails.
    struct FailingFlush(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl AsyncWrite for FailingFlush {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            let lines = buf.iter().filter(|&&b| b == b'\n').count();
            self.0.fetch_add(lines, std::sync::atomic::Ordering::SeqCst);
            std::task::Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::other("closed")))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// A client that can't take a flushed reply is gone: the writer stops
    /// after the first, rather than writing on into a dead channel.
    #[tokio::test]
    async fn a_failed_flush_stops_the_writer() {
        let written = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let input = std::io::Cursor::new(b"not json\nnor this\n".to_vec());
        let (rmcp_in, rmcp_out, writer) = spawn(input, FailingFlush(written.clone()));
        drop((rmcp_in, rmcp_out));
        writer.await.unwrap();
        assert_eq!(written.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
