use crate::daemon::MagiskD;
use crate::resetprop::get_prop;
use base::{cstr, debug, error, info, warn};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::Ordering;

const ENABLE_PROP: &str = "persist.magisk.magiskV_api";
const ADDR_PROP: &str = "persist.magisk.magiskV_api_addr";
const ALLOW_LAN_PROP: &str = "persist.magisk.magiskV_api_lan";
const TOKEN_PROP: &str = "persist.magisk.magiskV_api_token";
const DEFAULT_ADDR: &str = "127.0.0.1:48123";
const DEFAULT_LAN_ADDR: &str = "0.0.0.0:48123";

pub fn start_magiskV_api_if_enabled(daemon: &MagiskD) {
    let enabled = get_prop(cstr!(ENABLE_PROP)) == "1";
    if !enabled {
        debug!("magiskV_api: disabled by {ENABLE_PROP}");
        return;
    }

    if daemon.magiskV_api_started.swap(true, Ordering::AcqRel) {
        return;
    }

    let allow_lan = get_prop(cstr!(ALLOW_LAN_PROP)) == "1";
    let token = get_prop(cstr!(TOKEN_PROP));
    let addr = get_prop(cstr!(ADDR_PROP));
    debug!(
        "magiskV_api: config lan={} addr_prop='{}' token_set={}",
        allow_lan,
        addr,
        !token.is_empty()
    );
    let addr = if addr.is_empty() {
        if allow_lan {
            DEFAULT_LAN_ADDR.to_string()
        } else {
            DEFAULT_ADDR.to_string()
        }
    } else if allow_lan {
        addr
    } else if addr.starts_with("127.0.0.1:") || addr.starts_with("[::1]:") {
        addr
    } else {
        warn!("HTTP API address must be loopback, fallback to {DEFAULT_ADDR}");
        DEFAULT_ADDR.to_string()
    };

    if allow_lan && token.is_empty() {
        warn!("HTTP API LAN mode enabled without token (set {TOKEN_PROP})");
    }
    info!("* magiskV_api starting on {addr}");
    std::thread::spawn(move || run_http_server(addr, token));
}

fn run_http_server(addr: String, token: String) {
    let Ok(listener) = TcpListener::bind(&addr) else {
        error!("* HTTP API bind failed on {addr}");
        return;
    };
    info!("* HTTP API listening on {addr}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let token = token.clone();
                std::thread::spawn(move || handle_connection(stream, token));
            }
            Err(e) => {
                warn!("HTTP API accept failed: {e}");
            }
        }
    }
}

fn handle_connection(mut stream: TcpStream, token: String) {
    if let Ok(addr) = stream.peer_addr() {
        debug!("magiskV_api: connection from {}", addr);
    }
    let mut req = [0_u8; 8192];
    let Ok(n) = stream.read(&mut req) else {
        return;
    };
    if n == 0 {
        return;
    }
    let line = String::from_utf8_lossy(&req[..n]);
    let Some(first_line) = line.lines().next() else {
        write_response(&mut stream, 400, "Bad Request");
        return;
    };

    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    debug!("magiskV_api: request {} {}", method, target);
    let query = target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default();

    if !token.is_empty() {
        let ok = query_param(query, "token")
            .map(|v| url_decode(v) == token)
            .unwrap_or(false);
        if !ok {
            warn!("magiskV_api: rejected request due to invalid token");
            write_response(&mut stream, 403, "forbidden\n");
            return;
        }
    }

    if method != "GET" {
        warn!("magiskV_api: method not allowed: {method}");
        write_response(&mut stream, 405, "Only GET is supported\n");
        return;
    }

    if target == "/health" {
        debug!("magiskV_api: health check");
        write_response(&mut stream, 200, "ok\n");
        return;
    }

    let Some(cmd) = extract_cmd(target) else {
        warn!("magiskV_api: invalid endpoint {}", target);
        write_response(
            &mut stream,
            400,
            "usage: /cmd?cmd=pm%20list%20packages\n",
        );
        return;
    };

    let mut shell = Command::new("/system/bin/sh");
    shell.arg("-c").arg(&cmd);
    debug!("magiskV_api: exec cmd='{}'", cmd);
    let output = shell.output();
    match output {
        Ok(out) => {
            debug!(
                "magiskV_api: cmd exit={} stdout_bytes={} stderr_bytes={}",
                out.status.code().unwrap_or(-1),
                out.stdout.len(),
                out.stderr.len()
            );
            let mut body = Vec::new();
            body.extend_from_slice(format!("exit={}\n", out.status.code().unwrap_or(-1)).as_bytes());
            if !out.stdout.is_empty() {
                body.extend_from_slice(b"stdout:\n");
                body.extend_from_slice(&out.stdout);
                if !out.stdout.ends_with(b"\n") {
                    body.extend_from_slice(b"\n");
                }
            }
            if !out.stderr.is_empty() {
                body.extend_from_slice(b"stderr:\n");
                body.extend_from_slice(&out.stderr);
                if !out.stderr.ends_with(b"\n") {
                    body.extend_from_slice(b"\n");
                }
            }
            write_raw_response(&mut stream, 200, &body);
        }
        Err(e) => {
            write_response(&mut stream, 500, &format!("command execution failed: {e}\n"));
        }
    }
}

fn extract_cmd(target: &str) -> Option<String> {
    let (path, query) = target.split_once('?')?;
    if path != "/cmd" {
        return None;
    }
    query_param(query, "cmd").map(url_decode)
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k == key {
            return Some(v);
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        if b[i] == b'%' && i + 2 < b.len() {
            let h1 = (b[i + 1] as char).to_digit(16);
            let h2 = (b[i + 2] as char).to_digit(16);
            if let (Some(h1), Some(h2)) = (h1, h2) {
                out.push(((h1 * 16 + h2) as u8) as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn write_response(stream: &mut TcpStream, status: i32, body: &str) {
    write_raw_response(stream, status, body.as_bytes());
}

fn write_raw_response(stream: &mut TcpStream, status: i32, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
