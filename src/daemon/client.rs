//! The client seam — how a thin client learns whether the resident daemon is
//! reachable, and is FORCED to notice when it is not (aegis-1qze).
//!
//! This is the load-bearing safety piece of stage 1. When the guard becomes a
//! thin client of the daemon (stage 3), "the daemon did not answer" must never be
//! quietly treated as "allow" — killing the daemon would otherwise be the cheapest
//! way to disable the guard on every edit at once. So a probe returns
//! [`Reachability`], whose `Down` variant carries a reason and cannot be folded
//! into a success by accident: there is no `Default`, no `unwrap_or(allow)` shape,
//! and the type makes a caller pattern-match both arms.
//!
//! Deliberately dependency-free (`std::net` only, no async, no `reqwest`): the
//! pre-edit hook runs synchronously on stdin, so the probe it will call in stage 3
//! must work in that context. A raw HTTP/1.1 `GET /health` over a connect-timed
//! `TcpStream` is enough to answer the only question here — "is a hank daemon
//! listening and healthy at this address?"

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::MeasureReply;

/// Whether a resident daemon answered a liveness probe.
///
/// `Down` is a first-class outcome, not an error to be `?`-propagated away: the
/// whole point is that a caller must SEE it and decide loudly (fall back to a
/// transient build, and say the guard is running unguarded-by-daemon), never
/// silently allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// The daemon answered `/health` with a 2xx.
    Up,
    /// No healthy daemon at this address. The reason is for the operator-facing
    /// notice, not for control flow — every `Down`, whatever the reason, means
    /// "there is no resident guard here right now".
    Down(String),
}

impl Reachability {
    /// True only when a healthy daemon answered. Named so call sites read as a
    /// question, and so the `Down` case is never the implicit `else`.
    #[must_use]
    pub fn is_up(&self) -> bool {
        matches!(self, Reachability::Up)
    }

    /// The reason a probe came back `Down`, for a user-visible notice.
    #[must_use]
    pub fn down_reason(&self) -> Option<&str> {
        match self {
            Reachability::Up => None,
            Reachability::Down(why) => Some(why),
        }
    }
}

/// Probe `http://<host>:<port>/health`. Returns [`Reachability::Up`] only on a
/// 2xx; any connect error, timeout, or non-2xx is [`Reachability::Down`] with a
/// reason. Never panics, never blocks longer than `timeout` on connect.
#[must_use]
pub fn probe(host: &str, port: u16, timeout: Duration) -> Reachability {
    let addr = format!("{host}:{port}");
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return Reachability::Down(format!("cannot resolve {addr}"));
    };
    let Some(sockaddr) = addrs.next() else {
        return Reachability::Down(format!("no address for {addr}"));
    };

    let mut stream = match TcpStream::connect_timeout(&sockaddr, timeout) {
        Ok(s) => s,
        // Connection refused is the common, important case: no daemon is
        // listening. Named plainly so the notice reads "no daemon", not a raw
        // errno.
        Err(e) => return Reachability::Down(format!("no daemon at {addr} ({e})")),
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let req = format!("GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if let Err(e) = stream.write_all(req.as_bytes()) {
        return Reachability::Down(format!("write to {addr} failed ({e})"));
    }

    // The status line is all we need: `HTTP/1.1 200 ...`.
    let mut buf = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' || buf.len() >= 64 {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(e) => return Reachability::Down(format!("read from {addr} failed ({e})")),
        }
    }
    let line = String::from_utf8_lossy(&buf);
    // "HTTP/1.1 200 OK" -> the middle token is the status code.
    match line.split_whitespace().nth(1) {
        Some(code) if code.starts_with('2') => Reachability::Up,
        Some(code) => Reachability::Down(format!("daemon at {addr} returned HTTP {code}")),
        None => Reachability::Down(format!("daemon at {addr} sent no status line")),
    }
}

/// Ask the resident daemon to size an edit: `POST /measure`. Returns the
/// [`MeasureReply`] on a 2xx, or an `Err(reason)` on ANY other outcome —
/// connection refused (no daemon), a non-2xx (e.g. the file is outside the
/// daemon's root, so it serves a different repo), a timeout, or an unparseable
/// body. The caller MUST treat `Err` as "the daemon could not size this edit" and
/// fall back — never as "allow". Same `std::net`-only, synchronous shape as
/// [`probe`], because the pre-edit hook that calls this runs sync on stdin.
///
/// The reason string distinguishes the failures (a connection error reads
/// differently from an "HTTP 400"), so a caller can say WHY the daemon was
/// unusable in its loud notice — down vs. serving-another-repo are different
/// operator problems.
pub fn fetch_measure(
    host: &str,
    port: u16,
    file: &str,
    rel: &str,
    anchors: &[String],
    max_hops: u32,
    timeout: Duration,
) -> Result<MeasureReply, String> {
    let addr = format!("{host}:{port}");
    let body = serde_json::json!({
        "file": file,
        "rel": rel,
        "anchors": anchors,
        "max_hops": max_hops,
    })
    .to_string();

    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return Err(format!("cannot resolve {addr}"));
    };
    let Some(sockaddr) = addrs.next() else {
        return Err(format!("no address for {addr}"));
    };
    let mut stream = TcpStream::connect_timeout(&sockaddr, timeout)
        .map_err(|e| format!("no daemon at {addr} ({e})"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let req = format!(
        "POST /measure HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write to {addr} failed ({e})"))?;

    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .map_err(|e| format!("read from {addr} failed ({e})"))?;

    let status = raw
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("daemon at {addr} sent no status line"))?;
    if !status.starts_with('2') {
        return Err(format!("daemon at {addr} returned HTTP {status}"));
    }
    let payload = raw
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| format!("daemon at {addr} sent no body"))?;
    serde_json::from_str::<MeasureReply>(payload)
        .map_err(|e| format!("daemon at {addr} sent an unparseable measure reply ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_port_is_DOWN_with_a_reason_not_a_panic() {
        // The exact case that must never be silently "allow": nothing is
        // listening. Port 1 is reserved and never open.
        let r = probe("127.0.0.1", 1, Duration::from_millis(200));
        assert!(!r.is_up());
        assert!(
            r.down_reason().is_some(),
            "Down must carry a reason for the notice"
        );
    }

    #[test]
    fn an_unroutable_host_is_DOWN_not_UP() {
        // A resolve/connect failure is Down, never mistaken for Up.
        let r = probe(
            "no.such.host.hank.invalid",
            3040,
            Duration::from_millis(200),
        );
        assert!(!r.is_up());
    }

    #[test]
    fn down_and_up_are_distinct_and_reason_only_on_down() {
        assert!(Reachability::Up.is_up());
        assert_eq!(Reachability::Up.down_reason(), None);
        let d = Reachability::Down("no daemon".into());
        assert!(!d.is_up());
        assert_eq!(d.down_reason(), Some("no daemon"));
    }
}
