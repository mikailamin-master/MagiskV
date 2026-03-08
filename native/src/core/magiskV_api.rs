use crate::daemon::MagiskD;
use crate::consts::DEFAULT_ADDR;
use base::{debug, error, info, warn};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Default)]
struct SshState {
    child: Option<Child>,
    port: u16,
    daemon: String,
}

static SSH_STATE: OnceLock<Mutex<SshState>> = OnceLock::new();

fn ssh_state() -> &'static Mutex<SshState> {
    SSH_STATE.get_or_init(|| Mutex::new(SshState::default()))
}

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
    open_firewall_port(&port);

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

fn open_firewall_port(port: &str) {

    let cmd = format!(
        "iptables -I INPUT 1 -p tcp --dport {} -j ACCEPT; \
         iptables -I OUTPUT 1 -p tcp --sport {} -j ACCEPT",
        port, port
    );

    let _ = Command::new("/system/bin/sh")
        .arg("-c")
        .arg(cmd)
        .output();
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
        write_response(&mut stream, 400, "Bad Request");
        return;
    };

    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    debug!("magiskV_api: request {} {}", method, target);

    if method != "GET" {
        warn!("magiskV_api: method not allowed: {method}");
        write_response(&mut stream, 405, "Only GET supported\n");
        return;
    }

    if target == "/status" {
        write_response(&mut stream, 200, "ok\n");
        return;
    }

    if target.starts_with("/ssh/") {
        handle_ssh_api(&mut stream, target);
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

    debug!("magiskV_api: exec cmd='{}'", cmd);

    match shell.output() {
        Ok(out) => {

            let mut body = Vec::new();

            body.extend_from_slice(
                format!("exit={}\n", out.status.code().unwrap_or(-1)).as_bytes(),
            );

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
            write_response(
                &mut stream,
                500,
                &format!("command execution failed: {e}\n"),
            );
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

fn handle_ssh_api(stream: &mut TcpStream, target: &str) {
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    match path {
        "/ssh/status" => write_response(stream, 200, &ssh_status()),
        "/ssh/start" => {
            let query = target.split_once('?').map_or("", |(_, q)| q);
            let port = parse_port(query).unwrap_or(26267);
            match ssh_start(port) {
                Ok(msg) => write_response(stream, 200, &msg),
                Err(msg) => write_response(stream, 500, &msg),
            }
        }
        "/ssh/stop" => match ssh_stop() {
            Ok(msg) => write_response(stream, 200, &msg),
            Err(msg) => write_response(stream, 500, &msg),
        },
        _ => write_response(
            stream,
            400,
            "usage:\n/ssh/status\n/ssh/start?port=26267\n/ssh/stop\n",
        ),
    }
}

fn parse_port(query: &str) -> Option<u16> {
    query_param(query, "port")
        .map(url_decode)
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
}

fn api_port() -> u16 {
    get_port(DEFAULT_ADDR).parse::<u16>().unwrap_or(26266)
}

fn ssh_status() -> String {
    let mut state = ssh_state().lock().expect("ssh state poisoned");
    if let Some(child) = state.child.as_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                let msg = format!(
                    "stopped\nexit={}\n",
                    status.code().unwrap_or(-1)
                );
                state.child = None;
                state.port = 0;
                state.daemon.clear();
                msg
            }
            Ok(None) => {
                let pid = child.id();
                let port = state.port;
                let daemon = state.daemon.clone();
                format!(
                    "running\nport={}\npid={}\ndaemon={}\n",
                    port, pid, daemon
                )
            }
            Err(e) => format!("error\n{}\n", e),
        }
    } else {
        "stopped\n".to_string()
    }
}

fn ssh_start(port: u16) -> Result<String, String> {
    let reserved_port = api_port();
    if port == reserved_port {
        return Err(format!(
            "port {reserved_port} is reserved for magiskV HTTP API\n"
        ));
    }

    let mut state = ssh_state().lock().expect("ssh state poisoned");

    if let Some(child) = state.child.as_mut() {
        if let Ok(None) = child.try_wait() {
            let pid = child.id();
            let running_port = state.port;
            let daemon = state.daemon.clone();
            return Ok(format!(
                "already running\nport={}\npid={}\ndaemon={}\n",
                running_port, pid, daemon
            ));
        }
        state.child = None;
        state.port = 0;
        state.daemon.clear();
    }

    let script = format!(
        "if [ -x /data/adb/magisk/dropbear ]; then exec /data/adb/magisk/dropbear -R -E -F -p {port}; \
         elif command -v dropbear >/dev/null 2>&1; then exec \"$(command -v dropbear)\" -R -E -F -p {port}; \
         elif command -v sshd >/dev/null 2>&1; then exec \"$(command -v sshd)\" -D -p {port}; \
         else echo 'No SSH daemon found (dropbear/sshd)' >&2; exit 127; fi"
    );

    let mut child = Command::new("/system/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch shell: {e}\n"))?;

    std::thread::sleep(Duration::from_millis(400));

    if let Ok(Some(status)) = child.try_wait() {
        return Err(format!(
            "ssh daemon exited early\nexit={}\n",
            status.code().unwrap_or(-1)
        ));
    }

    let daemon = detect_ssh_daemon().unwrap_or_else(|| "unknown".to_string());
    let pid = child.id();
    state.child = Some(child);
    state.port = port;
    state.daemon = daemon.clone();

    Ok(format!(
        "started\nport={port}\npid={pid}\ndaemon={daemon}\n",
    ))
}

fn ssh_stop() -> Result<String, String> {
    let mut state = ssh_state().lock().expect("ssh state poisoned");
    let Some(mut child) = state.child.take() else {
        return Ok("already stopped\n".to_string());
    };

    let _ = child.kill();
    let _ = child.wait();
    state.port = 0;
    state.daemon.clear();
    Ok("stopped\n".to_string())
}

fn detect_ssh_daemon() -> Option<String> {
    if Command::new("/system/bin/sh")
        .arg("-c")
        .arg("[ -x /data/adb/magisk/dropbear ]")
        .status()
        .ok()?
        .success()
    {
        return Some("dropbear".to_string());
    }
    if Command::new("/system/bin/sh")
        .arg("-c")
        .arg("command -v dropbear >/dev/null 2>&1")
        .status()
        .ok()?
        .success()
    {
        return Some("dropbear".to_string());
    }
    if Command::new("/system/bin/sh")
        .arg("-c")
        .arg("command -v sshd >/dev/null 2>&1")
        .status()
        .ok()?
        .success()
    {
        return Some("sshd".to_string());
    }
    None
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
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );

    let _ = stream.write_all(body);
    let _ = stream.flush();
}
