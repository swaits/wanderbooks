//! The two pages a person's browser sees during a sign-in: "look at
//! Wanderbooks", and a plain problem. Plain words; the person may not know
//! what any of this is called, and the word is "assistant".
//!
//! ```text
//! GOAL: the browser page of a sign-in never approves anything and never
//!       carries text a stranger wrote unescaped.
//! SOURCE: WB-remote-5 (approval is the person's, in the app's window);
//!         plan/p15-app-and-relay.md 3.3, D-83.
//! INVARIANTS:
//!   1. Every piece of text from outside (a client's name, a company's
//!      name) is HTML-escaped; the page has no script, no form, no link
//!      and no button that approves: it says to look at Wanderbooks and
//!      looks again by itself (a refresh), until the person decides.
//!   2. The page's only address is the relative one of the waiting
//!      request; nothing is built from the request's Host.
//! ```

/// Escape text for HTML.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn shell(title: &str, refresh: Option<&str>, body: &str) -> String {
    let head = refresh
        .map(|u| format!("<meta http-equiv=\"refresh\" content=\"2;url={}\">", esc(u)))
        .unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"referrer\" content=\"no-referrer\">{head}<title>{}</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1rem;color:#222}}\
         h1{{font-size:1.4rem}}p{{color:#444}}</style></head><body>{body}</body></html>",
        esc(title)
    )
}

/// The page while the person has not decided: look at the app.
pub fn waiting(client: &str, company: &str, wait: &str) -> String {
    shell(
        "Look at Wanderbooks",
        Some(&format!("/authorize/wait/{wait}")),
        &format!(
            "<h1>Look at Wanderbooks</h1>\
             <p><strong>{}</strong> is asking to connect to <strong>{}</strong>.</p>\
             <p>Open the Wanderbooks window on your computer and choose <strong>Allow</strong> \
             or <strong>Don't allow</strong>. This page continues by itself when you do.</p>\
             <p>If you didn't ask an assistant to connect, choose Don't allow, or just close this page.</p>",
            esc(client),
            esc(company)
        ),
    )
}

/// A plain problem, with nothing to click.
pub fn problem(title: &str, text: &str) -> String {
    shell(
        title,
        None,
        &format!("<h1>{}</h1><p>{}</p>", esc(title), esc(text)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strangers_words_never_become_markup() {
        let page = waiting("<script>alert(1)</script>", "\"><img src=x>", "w_abc");
        assert!(!page.contains("<script>alert"));
        assert!(!page.contains("<img src=x>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("<form"));
        assert!(page.contains("url=/authorize/wait/w_abc"));
    }
}
