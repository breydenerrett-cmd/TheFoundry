//! L-01: tiny dependency-free localhost HTTP/1.1 server for `--serve`.
//! `GET /state` returns the latest exported `FloorState` JSON; `GET /health`
//! returns `{"ok":true,"seq":N}`; anything else 404s. Refuses to bind any
//! host but loopback (127.0.0.1 / ::1) — this is a local dev bridge, never a
//! public listener.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone)]
pub struct ServedState {
    pub json: Arc<Mutex<String>>,
    pub seq: Arc<AtomicU64>,
    /// When set, `X-Foundry-Token` must match exactly or the request is 401'd.
    pub token: Option<String>,
}

/// Binds `addr` (loopback only — anything else is refused with an error) and
/// spawns a thread that serves connections forever. Returns the bound
/// `TcpListener` so callers (tests included) can read the actual local port,
/// e.g. when binding to port 0.
pub fn serve(addr: SocketAddr, state: ServedState) -> std::io::Result<TcpListener> {
    let loopback = matches!(addr.ip(), IpAddr::V4(v4) if v4 == Ipv4Addr::LOCALHOST)
        || matches!(addr.ip(), IpAddr::V6(v6) if v6 == Ipv6Addr::LOCALHOST);
    if !loopback {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to bind --serve to a non-loopback host",
        ));
    }
    let listener = TcpListener::bind(addr)?;
    let accept_loop = listener.try_clone()?;
    thread::spawn(move || {
        for stream in accept_loop.incoming().flatten() {
            let state = state.clone();
            thread::spawn(move || {
                let _ = handle(stream, &state);
            });
        }
    });
    Ok(listener)
}

fn handle(mut stream: TcpStream, state: &ServedState) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut origin: Option<String> = None;
    let mut token_header: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            break;
        }
        let line = line.trim_end();
        if let Some(v) = line.strip_prefix("Origin:") {
            origin = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("X-Foundry-Token:") {
            token_header = Some(v.trim().to_string());
        }
    }

    // Access-Control-Allow-Origin: * is only ever set when the request
    // itself came from a localhost-ish origin — never a blanket CORS grant.
    let cors = origin
        .as_deref()
        .map(|o| o.contains("127.0.0.1") || o.contains("localhost") || o.contains("[::1]"))
        .unwrap_or(false);

    if method != "GET" {
        return respond(&mut stream, 404, "text/plain", "not found", cors);
    }
    if let Some(expected) = &state.token {
        if token_header.as_deref() != Some(expected.as_str()) {
            return respond(&mut stream, 401, "text/plain", "unauthorized", cors);
        }
    }
    match path.as_str() {
        "/state" => {
            let body = state.json.lock().unwrap().clone();
            respond(&mut stream, 200, "application/json", &body, cors)
        }
        "/health" => {
            let seq = state.seq.load(Ordering::SeqCst);
            let body = format!("{{\"ok\":true,\"seq\":{seq}}}");
            respond(&mut stream, 200, "application/json", &body, cors)
        }
        _ => respond(&mut stream, 404, "text/plain", "not found", cors),
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
    cors: bool,
) -> std::io::Result<()> {
    let status_text = match status {
        200 => "OK",
        401 => "Unauthorized",
        _ => "Not Found",
    };
    let mut resp = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if cors {
        resp.push_str("Access-Control-Allow-Origin: *\r\n");
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    stream.write_all(resp.as_bytes())
}
