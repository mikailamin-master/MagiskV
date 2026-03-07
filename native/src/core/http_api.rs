use crate::daemon::MagiskD;
use crate::resetprop::get_prop;
use base::{error, info, warn, cstr};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::Ordering;

const ENABLE_PROP: &str = "persist.magisk.http_api";
const ADDR_PROP: &str = "persist.magisk.http_api_addr";
const DEFAULT_ADDR: &str = "127.0.0.1:48123";

pub fn start_http_api_if_enabled(daemon: &MagiskD) {
    let enabled = get_prop(cstr!(ENABLE_PROP)) == "1";
    if !enabled {
        return;
    }

    if daemon.http_api_started.swap(true, Ordering::AcqRel) {
        return;
    }

    let addr = get_prop(cstr!(ADDR_PROP));
    let addr = if addr.is_empty() {
        DEFAULT_ADDR.to_string()
    } else if addr.starts_with("127.0.0.1:") || addr.starts_with("[::1]:") {
        addr
    } else {
        warn!("HTTP API address must be loopback, fallback to {DEFAULT_ADDR}");
        DEFAULT_ADDR.to_string()
    };

    std::thread::spawn(move || run_http_server(addr));
}

fn run_http_server(addr: String) {
    let Ok(listener) = TcpListener::bind(&addr) else {
        error!("* HTTP API bind failed on {addr}");
        return;
    };
    info!("* HTTP API listening on {addr}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || handle_connection(stream));
            }
            Err(e) => {
                warn!("HTTP API accept failed: {e}");
            }
        }
    }
}

fn handle_connection(mut stream: TcpStream) {
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
    if method != "GET" {
        write_response(&mut stream, 405, "Only GET is supported\n");
        return;
    }

    if target == "/health" {
        write_response(&mut stream, 200, "ok\n");
        return;
    }

    let Some(cmd) = extract_cmd(target) else {
        write_response(
            &mut stream,
            400,
            "usage: /cmd?cmd=pm%20list%20packages\n",
        );
        return;
    };

    let mut shell = Command::new("/system/bin/sh");
    shell.arg("-c").arg(&cmd);
    let output = shell.output();
    match output {
        Ok(out) => {
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
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k == "cmd" {
            return Some(url_decode(v));
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
