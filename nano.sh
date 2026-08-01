#!/usr/bin/env bash
#
# nano.sh - Edit a remote file over a MagiskV shell.
#
# The MagiskV daemon (Magisk) listens on port 26268 and exposes a shell over a
# plain TCP socket. This script uses that shell to download a remote file
# (base64-encoded, so binary files are safe), opens it in $EDITOR (or nano),
# and uploads the modified contents back when the editor exits.
#
# Usage:
#     ./nano.sh <host> <remote_path>            # default port 26268
#     ./nano.sh <host> <port> <remote_path>     # explicit port
#     EDITOR=vim ./nano.sh 192.168.1.10 /etc/hosts
#
# Requires: bash, nc (netcat) or /dev/tcp, base64, timeout.
#

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
REMOTE_PATH=""
HOST=""
PORT="${MAGISKV_PORT:-26268}"

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 <host> <remote_path> [port]" >&2
    echo "       $0 <host> <port> <remote_path>   (use MAGISKV_PORT)" >&2
    exit 1
fi

HOST="$1"
if [[ $# -ge 3 ]]; then
    case "$2" in
        ''|*[!0-9]*)
            # $2 is not a number -> treat as remote path (port from env)
            REMOTE_PATH="$2"
            ;;
        *)
            PORT="$2"
            REMOTE_PATH="$3"
            ;;
    esac
else
    REMOTE_PATH="$2"
fi

if [[ -z "$REMOTE_PATH" ]]; then
    echo "Error: remote_path is required." >&2
    exit 1
fi

EDITOR="${EDITOR:-nano}"

# ---------------------------------------------------------------------------
# Open a raw TCP connection to the remote shell.
# ---------------------------------------------------------------------------
open_connection() {
    if command -v nc >/dev/null 2>&1; then
        # -N to close on EOF (GNU netcat). -q fallback for older versions.
        if nc -h 2>&1 | grep -q -- '-N'; then
            exec 3<>"/dev/tcp/${HOST}/${PORT}"
        else
            exec 3<>"/dev/tcp/${HOST}/${PORT}"
        fi
    else
        exec 3<>"/dev/tcp/${HOST}/${PORT}"
    fi
}

# Prefer bash /dev/tcp which is universally available with bash and supports
# a full duplex fd (3) we can both read and write to.
if { exec 3<>"/dev/tcp/${HOST}/${PORT}"; } 2>/dev/null; then
    :
else
    echo "! Could not connect to ${HOST}:${PORT} (no /dev/tcp support)." >&2
    exit 1
fi

echo "* Connected to ${HOST}:${PORT}" >&2

# Drain any initial banner/prompt so it is not mistaken for command output.
sleep 0.3
read -r -t 0.5 -u 3 _banner || true

# ---------------------------------------------------------------------------
# Send a command to the remote shell and capture its full stdout.
# Sentinels wrap the output so we can reliably find the start/end.
# ---------------------------------------------------------------------------
BEGIN="___MV_BEGIN_5f9a8c3e___"
END="___MV_END_5f9a8c3e___"

run_remote() {
    local cmd="$1"
    printf 'echo "%s"; %s; echo "%s"\n' "$BEGIN" "$cmd" "$END" >&3
    # Read until we see the END marker.
    local out=""
    local line
    while IFS= read -r -t 60 line <&3; do
        if [[ "$line" == "$END" ]]; then
            break
        fi
        # Skip the BEGIN marker line and anything before it on noisy shells.
        if [[ "$line" == "$BEGIN" ]]; then
            continue
        fi
        out+="$line"$'\n'
    done
    printf '%s' "$out"
}

# ---------------------------------------------------------------------------
# Download the remote file (base64) and decode it locally.
# ---------------------------------------------------------------------------
TMP_FILE="$(mktemp)"
trap 'rm -f "$TMP_FILE"; exec 3>&- 2>/dev/null || true' EXIT

echo "* Downloading remote file: $REMOTE_PATH" >&2
b64="$(run_remote "base64 -w 0 ${REMOTE_PATH}")"

if [[ -z "$b64" ]]; then
    echo "! Remote command returned no data. The path may not exist or base64 is unavailable." >&2
    exit 1
fi

printf '%s' "$b64" | base64 -d >"$TMP_FILE" 2>/dev/null \
    || { echo "! Failed to decode base64 from remote." >&2; exit 1; }

local_size="$(wc -c <"$TMP_FILE")"
echo "* Downloaded ${local_size} bytes -> $TMP_FILE" >&2

# ---------------------------------------------------------------------------
# Let the user edit locally.
# ---------------------------------------------------------------------------
echo "* Opening ${EDITOR} for local editing (saves upload back to ${HOST}:${REMOTE_PATH})" >&2
command -v "$EDITOR" >/dev/null 2>&1 || {
    echo "! Editor '$EDITOR' not found; falling back to vi" >&2
    EDITOR="vi"
}
"$EDITOR" "$TMP_FILE" || true

# ---------------------------------------------------------------------------
# Upload the modified file back over the shell via base64.
# ---------------------------------------------------------------------------
new_b64="$(base64 -w 0 "$TMP_FILE" 2>/dev/null || base64 "$TMP_FILE" 2>/dev/null)"
echo "* Uploading modified file (${new_b64:+${#new_b64}} base64 chars)" >&2

# Build a single shell line that decodes and writes the file atomically.
printf 'printf %%s %q | base64 -d > "%s" && echo UPLOAD_OK || echo UPLOAD_FAIL\n' \
    "$new_b64" "$REMOTE_PATH" >&3

upload_status=""
while IFS= read -r -t 60 line <&3; do
    case "$line" in
        UPLOAD_OK|UPLOAD_FAIL)
            upload_status="$line"
            break
            ;;
    esac
done

if [[ "$upload_status" == "UPLOAD_OK" ]]; then
    echo "* Upload complete. Remote file updated." >&2
else
    echo "! Upload may have failed. Status: ${upload_status:-<timeout>}" >&2
fi

echo "* Done." >&2
