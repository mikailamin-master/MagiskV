#!/usr/bin/env python3
"""
MagiskV.py - MagiskV Remote Access Shell and utilities.

Usage:
    MagiskV.py <host> [port]          Interactive remote shell (default mode)
    MagiskV.py --scan                 Discover MagiskV daemons on the LAN
    MagiskV.py --nano <remote> [path] Edit a remote file with $EDITOR

The remote MagiskV daemon is expected to listen on port 26268 by default and
expose a plain shell that forwards stdin/stdout over the TCP socket.
"""

import argparse
import base64
import ipaddress
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

DEFAULT_PORT = 26268
BANNER = "MagiskV Remote Access Shell (use Ctrl+C or 'exit' to disconnect)"

# Unique sentinels used to delimit command output over the shared shell stream.
# They are designed so that they will not appear in normal command output.
START_MARKER = "___MV_BEGIN_5f9a8c3e___"
END_MARKER = "___MV_END_5f9a8c3e___"

SCAN_TIMEOUT = 0.3          # seconds per host connect attempt
SCAN_THREADS = 200          # concurrent probe workers


def spinner(msg):
    sys.stderr.write(f"\r* {msg}")
    sys.stderr.flush()


# --------------------------------------------------------------------------
# Interactive shell mode
# --------------------------------------------------------------------------


def receive_output(sock):
    try:
        while True:
            data = sock.recv(4096)
            if not data:
                break
            sys.stdout.buffer.write(data)
            sys.stdout.buffer.flush()
    except (ConnectionResetError, BrokenPipeError, OSError):
        pass
    finally:
        sys.stderr.write("\n! Connection closed by remote host\n")
        sys.exit(1)


def shell_mode(host, port):
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(10)
    try:
        spinner(f"Connecting to {host}:{port}")
        sock.connect((host, port))
        sock.settimeout(None)
        sys.stderr.write(f"\r* Connected to {host}:{port}\n")
        sys.stderr.write(f"{BANNER}\n")
        sys.stderr.write("-" * 50 + "\n")
    except Exception as e:
        sys.stderr.write(f"\r! Connection failed: {e}\n")
        sys.exit(1)

    signal.signal(signal.SIGINT, lambda s, f: sys.exit(0))

    recv_thread = threading.Thread(target=receive_output, args=(sock,), daemon=True)
    recv_thread.start()

    try:
        for line in sys.stdin.buffer:
            sock.sendall(line)
    except (BrokenPipeError, OSError):
        pass
    finally:
        sock.close()


# --------------------------------------------------------------------------
# Command-execution helper over the shared shell stream
# --------------------------------------------------------------------------


def run_remote_command(sock, command, timeout=15):
    """Send a command to the remote shell and return its stdout as bytes.

    The command is wrapped with unique sentinel markers printed to stdout so
    that we can reliably detect the end of output even on a noisy shell.
    """
    wrapped = f'echo "{START_MARKER}"; {command}; echo "{END_MARKER}"\n'
    sock.sendall(wrapped.encode())

    buf = bytearray()
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            sock.settimeout(1.0)
            chunk = sock.recv(8192)
        except socket.timeout:
            continue
        except (ConnectionResetError, BrokenPipeError, OSError):
            break
        if not chunk:
            break
        buf.extend(chunk)
        if END_MARKER.encode() in buf:
            break

    text = buf.decode("utf-8", errors="replace")
    if START_MARKER in text and END_MARKER in text:
        start_idx = text.index(START_MARKER) + len(START_MARKER)
        end_idx = text.index(END_MARKER)
        return text[start_idx:end_idx]
    return text


# --------------------------------------------------------------------------
# --scan mode
# --------------------------------------------------------------------------


def get_local_subnet():
    """Best-effort discovery of the local IPv4 subnet (e.g. 192.168.1.0/24)."""
    try:
        # Use a UDP socket: no traffic is actually sent for this connect().
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.settimeout(0.5)
        s.connect(("8.8.8.8", 80))
        local_ip = s.getsockname()[0]
        s.close()
    except OSError:
        local_ip = "127.0.0.1"

    if local_ip.startswith("127.") or local_ip == "0.0.0.0":
        # Fallback: inspect interface addresses directly.
        try:
            local_ip = socket.gethostbyname(socket.gethostname())
        except OSError:
            return [], []

    # Guess a /24 by default; allow override via environment for speed.
    prefix_len = int(os.environ.get("MAGISKV_SCAN_PREFIX", "24"))
    try:
        net = ipaddress.ip_network(f"{local_ip}/{prefix_len}", strict=False)
    except ValueError:
        net = ipaddress.ip_network(f"{local_ip}/24", strict=False)

    hosts = list(net.hosts())
    return local_ip, hosts


def probe_host(host, port, results):
    """Quick TCP connect probe. Appends the host to *results* on success."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.settimeout(SCAN_TIMEOUT)
            if s.connect_ex((host, port)) == 0:
                results.append(host)
    except OSError:
        pass


def scan_mode(port):
    local_ip, hosts = get_local_subnet()
    if not hosts:
        sys.stderr.write("! Could not determine local subnet. "
                         "Set MAGISKV_SCAN_PREFIX or specify via <host>.\n")
        sys.exit(1)

    sys.stderr.write(f"* Local IP: {local_ip}\n")
    sys.stderr.write(f"* Scanning {len(hosts)} hosts on port {port} "
                     f"(~{SCAN_TIMEOUT*1000:.0f}ms timeout, "
                     f"{SCAN_THREADS} threads)...\n")

    found = []
    found_lock = threading.Lock()
    threads = []
    pool_sema = threading.Semaphore(SCAN_THREADS)

    def worker(h):
        with pool_sema:
            probe_host(h, port, found)

    t0 = time.time()
    for h in hosts:
        t = threading.Thread(target=worker, args=(str(h),), daemon=True)
        threads.append(t)
        t.start()
    for t in threads:
        t.join()

    elapsed = time.time() - t0

    with found_lock:
        if local_ip in found:
            found.remove(local_ip)

    print("=" * 55)
    print(f" Scan complete in {elapsed:.2f}s")
    print("=" * 55)
    if found:
        print(f"Found {len(found)} MagiskV daemon(s):")
        for ip in found:
            print(f"  - {ip}:{port}")
    else:
        print("No MagiskV daemons found on the network.")
        print("Tip: run 'MagiskV.py --scan' on the same LAN segment.")
    return 0 if found else 1


# --------------------------------------------------------------------------
# --nano mode
# --------------------------------------------------------------------------


def nano_mode(remote, port, remote_path):
    """Download *remote_path* from the remote shell, edit it locally, re-upload.

    Uses base64 transport so that binary files are handled safely and the
    shell prompt noise does not corrupt the payload.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(10)
    try:
        spinner(f"Connecting to {remote}:{port}")
        sock.connect((remote, port))
        sock.settimeout(None)
        sys.stderr.write(f"\r* Connected to {remote}:{port}\n")
    except Exception as e:
        sys.stderr.write(f"\r! Connection failed: {e}\n")
        sys.exit(1)

    # Receive the initial shell banner / prompt so it is not mistaken for
    # command output later.
    time.sleep(0.3)
    try:
        sock.settimeout(0.5)
        sock.recv(8192)
    except OSError:
        pass

    # --- Download ---
    spinner(f"Downloading remote file: {remote_path}")
    b64 = run_remote_command(sock, f"base64 -w 0 {remote_path}", timeout=30)
    try:
        data = base64.b64decode(b64)
    except Exception:
        sys.stderr.write(f"\r! Could not decode remote file '{remote_path}'. "
                         f"It may not exist or base64 is unavailable.\n")
        sock.close()
        sys.exit(1)

    tmp = tempfile.NamedTemporaryFile(
        mode="wb", suffix="_magiskv_nano", delete=False
    )
    tmp.write(data)
    tmp.close()
    local_path = tmp.name
    sys.stderr.write(f"\r* Downloaded {len(data)} bytes -> {local_path}\n")

    # --- Edit locally ---
    editor = os.environ.get("EDITOR", "nano")
    sys.stderr.write(f"* Opening {editor} for local editing "
                     f"(saves will upload back to {remote}:{remote_path})\n")
    try:
        subprocess.run([editor, local_path], check=True)
    except FileNotFoundError:
        sys.stderr.write(f"! Editor '{editor}' not found; falling back to 'vi'\n")
        subprocess.run(["vi", local_path], check=True)
    except subprocess.CalledProcessError:
        pass

    # --- Upload ---
    with open(local_path, "rb") as f:
        new_data = f.read()
    b64_payload = base64.b64encode(new_data).decode("ascii")

    spinner("Uploading modified file")
    upload_cmd = (
        f"echo '{b64_payload}' | base64 -d > {remote_path} "
        f"&& echo UPLOAD_OK || echo UPLOAD_FAIL"
    )
    result = run_remote_command(sock, upload_cmd, timeout=60)
    if "UPLOAD_OK" in result:
        sys.stderr.write("\r* Upload complete. Remote file updated.\n")
    else:
        sys.stderr.write("\r! Upload may have failed. Remote output:\n")
        sys.stderr.write(result)

    sock.close()
    # Clean up the temp file.
    try:
        os.unlink(local_path)
    except OSError:
        pass
    return 0


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def parse_args():
    parser = argparse.ArgumentParser(
        prog="MagiskV.py",
        description="MagiskV remote access client, scanner, and editor.",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--scan", action="store_true",
                      help="scan the LAN for MagiskV daemons")
    mode.add_argument(
        "--nano",
        metavar="REMOTE_PATH",
        help="edit a remote file via a local $EDITOR (requires <host> <port>)",
    )
    parser.add_argument(
        "--port", type=int, default=DEFAULT_PORT,
        help=f"remote port (default: {DEFAULT_PORT})",
    )
    parser.add_argument(
        "host", nargs="?",
        help="remote host (host:port or host, with optional port)",
    )
    parser.add_argument(
        "port_arg", nargs="?",
        help="optional extra positional port (host <port>)",
    )
    return parser.parse_args()


def resolve_host_port(args):
    """Resolve host and port from the various accepted argument forms."""
    port = args.port
    host = args.host

    if args.nano is None and not args.scan:
        if host is None:
            print(f"Usage: {sys.argv[0]} <host> [port]")
            print(f"       {sys.argv[0]} <host>:<port>")
            print(f"       {sys.argv[0]} --scan")
            print(f"       {sys.argv[0]} --nano REMOTE_PATH <host> [port]")
            sys.exit(1)

        if host and ":" in host and host.count(":") == 1:
            h, p = host.split(":")
            host = h
            port = int(p)
        elif args.port_arg:
            port = int(args.port_arg)

    return host, port


def main():
    args = parse_args()

    if args.scan:
        sys.exit(scan_mode(args.port))

    host, port = resolve_host_port(args)

    if args.nano:
        sys.exit(nano_mode(host, port, args.nano))

    shell_mode(host, port)


if __name__ == "__main__":
    main()
