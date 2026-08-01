#!/usr/bin/env bash
#
# scan.sh - Discover MagiskV daemons on the local LAN.
#
# The MagiskV daemon (Magisk) listens on port 26268 by default. This script
# enumerates every host on the local /24 subnet and performs a fast parallel
# TCP connect-probe against that port. Any host that accepts the connection is
# reported as a MagiskV daemon.
#
# Usage:
#     ./scan.sh                  # scan on the default port (26268)
#     ./scan.sh 27555            # scan on an alternate port
#     MAGISKV_SCAN_PREFIX=16 ./scan.sh   # scan a /16 (limited to 2000 hosts)
#     ./scan.sh --subnet 192.168.1       # scan a specific /24 (no auto-detect)
#
# Requires: bash, nc (netcat) preferably, or /dev/tcp fallback, plus `timeout`.
#

set -euo pipefail

PORT="${DEFAULT_PORT:-26268}"
PREFIX_LEN="${MAGISKV_SCAN_PREFIX:-24}"
TIMEOUT_MS=300          # per-host connect timeout (ms)
MAX_PARALLEL=200        # max concurrent background probes
SUBNET_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --subnet)
            SUBNET_OVERRIDE="$2"
            shift 2
            ;;
        *)
            PORT="$1"
            shift
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------
if command -v nc >/dev/null 2>&1; then
    HAVE_NC=1
else
    HAVE_NC=0
fi

if ! command -v timeout >/dev/null 2>&1; then
    echo "! 'timeout' command not found; parallel probes will lack per-host timeouts." >&2
fi

# ---------------------------------------------------------------------------
# Determine the local IP.
# ---------------------------------------------------------------------------
get_local_ip() {
    if [[ -n "$SUBNET_OVERRIDE" ]]; then
        # Derive a plausible local IP from the override (/24 only).
        echo "${SUBNET_OVERRIDE}.1"
        return
    fi
    if command -v ip >/dev/null 2>&1; then
        ip route get 8.8.8.8 2>/dev/null | awk '{print $7; exit}' 2>/dev/null || true
    fi
    if [[ -z "${LOCAL_IP:-}" ]]; then
        hostname -I 2>/dev/null | awk '{print $1}' 2>/dev/null || true
    fi
}

LOCAL_IP="$(get_local_ip)"
if [[ -z "${LOCAL_IP}" ]]; then
    echo "! Could not determine local IP address." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Compute subnet hosts (pure bash integer math).
# ---------------------------------------------------------------------------
compute_range() {
    local prefix="$1"
    local ip="$LOCAL_IP"
    if [[ -n "$SUBNET_OVERRIDE" ]]; then
        ip="${SUBNET_OVERRIDE}.0"
    fi
    local a b c d
    IFS=. read -r a b c d <<<"$ip"
    local ip_int=$(( (a << 24) | (b << 16) | (c << 8) | d ))
    local mask=$(( (0xFFFFFFFF << (32 - prefix)) & 0xFFFFFFFF ))
    local net_int=$(( ip_int & mask ))
    local start=$(( net_int + 1 ))
    local end=$(( net_int | (~mask & 0xFFFFFFFF) ))
    echo "$start"
    echo "$end"
}

mapfile -t RANGE <<<"$(compute_range "$PREFIX_LEN")"
START_INT="${RANGE[0]}"
END_INT="${RANGE[1]}"

count=$(( END_INT - START_INT + 1 ))
if (( count > 2000 )); then
    echo "! Subnet prefix /${PREFIX_LEN} is too large (>2000 hosts). " \
         "Set MAGISKV_SCAN_PREFIX=24 (or smaller) or use --subnet." >&2
    exit 1
fi

echo "* Local IP: ${LOCAL_IP}" >&2
echo "* Scanning ${count} hosts on port ${PORT} " \
     "(~${TIMEOUT_MS}ms timeout, /${PREFIX_LEN}, ${MAX_PARALLEL} threads)..." >&2

# ---------------------------------------------------------------------------
# Probe a single IP. Writes "ip" to the results file if the port is open.
# ---------------------------------------------------------------------------
RESULTS_FILE="$(mktemp)"
trap 'rm -f "$RESULTS_FILE"' EXIT

probe() {
    local ip="$1"
    local ok=0
    if [[ "$HAVE_NC" == "1" ]]; then
        if nc -z -w "${TIMEOUT_MS}" "$ip" "$PORT" >/dev/null 2>&1; then
            ok=1
        fi
    else
        if timeout -s KILL 1 bash -c \
            "exec 3<>/dev/tcp/${ip}/${PORT}" >/dev/null 2>&1; then
            ok=1
        fi
    fi
    if [[ "$ok" == "1" ]]; then
        echo "$ip" >>"$RESULTS_FILE"
    fi
}

# ---------------------------------------------------------------------------
# Run probes in parallel with a bounded worker pool.
# ---------------------------------------------------------------------------
running=0
for int_ip in $(seq "$START_INT" "$END_INT"); do
    a=$(( (int_ip >> 24) & 0xFF ))
    b=$(( (int_ip >> 16) & 0xFF ))
    c=$(( (int_ip >> 8) & 0xFF ))
    d=$(( int_ip & 0xFF ))
    ip="${a}.${b}.${c}.${d}"

    # Skip our own IP.
    [[ "$ip" == "$LOCAL_IP" ]] && continue

    probe "$ip" &
    running=$((running + 1))
    if (( running >= MAX_PARALLEL )); then
        wait -n 2>/dev/null || wait
        running=$((running - 1))
    fi
done

# Drain remaining jobs.
wait

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
mapfile -t FOUND < <(sort -t. -k1,1n -k2,2n -k3,3n -k4,4n "$RESULTS_FILE" 2>/dev/null)

echo "=========================================================="
echo " Scan complete"
echo "=========================================================="
if (( ${#FOUND[@]} > 0 )); then
    echo "Found ${#FOUND[@]} MagiskV daemon(s):"
    for ip in "${FOUND[@]}"; do
        [[ -n "$ip" ]] && echo "  - ${ip}:${PORT}"
    done
    exit 0
else
    echo "No MagiskV daemons found on the network."
    echo "Tip: run 'MagiskV.py --scan' on the same LAN segment."
    exit 1
fi
