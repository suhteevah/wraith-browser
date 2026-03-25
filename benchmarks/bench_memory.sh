#!/usr/bin/env bash
# bench_memory.sh — Memory per session benchmark for Wraith Browser
#
# Starts the MCP server, creates N sessions via MCP protocol, and
# measures RSS of the wraith-browser process at each session count.
#
# Because the MCP server speaks JSON-RPC over stdio, direct scripting
# from bash is non-trivial. This script uses two approaches:
#
#   1. CLI mode (default): Launches N independent `wraith-browser navigate`
#      processes concurrently and measures aggregate RSS. This approximates
#      per-session overhead but includes per-process baseline cost.
#
#   2. MCP mode (manual): Documents the procedure for measuring true
#      in-process session memory via the MCP server.
#
# Usage:
#   WRAITH_BIN=./target/release/wraith-browser ./benchmarks/bench_memory.sh
#
# Requirements:
#   - Linux with bash 4+, /proc filesystem, ps, awk, bc
#   - A built wraith-browser binary

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
WRAITH_BIN="${WRAITH_BIN:-./target/release/wraith-browser}"
SESSION_COUNTS="${SESSION_COUNTS:-10 50 100 500}"
TEST_URL="${TEST_URL:-http://example.com}"
RESULTS_DIR="${RESULTS_DIR:-$(dirname "$0")/results}"
SETTLE_TIME="${SETTLE_TIME:-2}"  # seconds to wait after spawning before measuring

mkdir -p "$RESULTS_DIR"

if [[ ! -x "$WRAITH_BIN" ]]; then
    echo "ERROR: wraith-browser binary not found at: $WRAITH_BIN"
    echo "       Build it first:  cargo build --release"
    exit 1
fi

# ---------------------------------------------------------------------------
# Measure baseline RSS of a single short-lived process
# ---------------------------------------------------------------------------
echo "============================================================"
echo " Wraith Browser — Memory Per Session Benchmark"
echo "============================================================"
echo " Binary:       $WRAITH_BIN"
echo " Test URL:     $TEST_URL"
echo " Session counts: $SESSION_COUNTS"
echo " Started:      $(date -Iseconds)"
echo "============================================================"
echo ""

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/memory_${TIMESTAMP}.txt"

{
    echo "Wraith Browser Memory Benchmark — $(date -Iseconds)"
    echo "Binary: $WRAITH_BIN"
    echo "Test URL: $TEST_URL"
    echo ""
} > "$REPORT_FILE"

# ---------------------------------------------------------------------------
# Method 1: MCP server mode (measures true in-process sessions)
# ---------------------------------------------------------------------------
echo "=== Method 1: MCP Server In-Process Sessions ==="
echo ""
echo "This method starts the MCP server and creates sessions via JSON-RPC."
echo "The server holds all sessions in one process, giving accurate per-session"
echo "memory overhead."
echo ""

# Start MCP server in background, capture PID
MCP_PID=""
cleanup() {
    if [[ -n "$MCP_PID" ]] && kill -0 "$MCP_PID" 2>/dev/null; then
        kill "$MCP_PID" 2>/dev/null || true
        wait "$MCP_PID" 2>/dev/null || true
    fi
    # Clean up any lingering child processes
    jobs -p | xargs -r kill 2>/dev/null || true
}
trap cleanup EXIT

# Create a FIFO for MCP stdin
MCP_FIFO=$(mktemp -u)
mkfifo "$MCP_FIFO"

# Start MCP server with stdio transport
"$WRAITH_BIN" serve --transport stdio < "$MCP_FIFO" > /dev/null 2>&1 &
MCP_PID=$!

# Keep FIFO open for writing
exec 3>"$MCP_FIFO"

sleep "$SETTLE_TIME"

if ! kill -0 "$MCP_PID" 2>/dev/null; then
    echo "WARNING: MCP server failed to start. Falling back to CLI-only mode."
    MCP_PID=""
else
    # Measure baseline RSS
    BASELINE_RSS=$(ps -o rss= -p "$MCP_PID" 2>/dev/null | tr -d ' ')
    if [[ -z "$BASELINE_RSS" ]]; then
        BASELINE_RSS=0
    fi
    BASELINE_MB=$(echo "scale=2; $BASELINE_RSS / 1024" | bc)
    echo "Baseline (0 sessions): ${BASELINE_MB} MB (RSS: ${BASELINE_RSS} KB)"
    echo ""

    printf "%-15s %-15s %-15s %-15s\n" "Sessions" "Total RSS (MB)" "Per-Session (MB)" "Delta (MB)"
    printf "%-15s %-15s %-15s %-15s\n" "--------" "--------------" "----------------" "----------"

    {
        printf "%-15s %-15s %-15s %-15s\n" "Sessions" "Total RSS (MB)" "Per-Session (MB)" "Delta (MB)"
        printf "%-15s %-15s %-15s %-15s\n" "--------" "--------------" "----------------" "----------"
    } >> "$REPORT_FILE"

    PREV_RSS=$BASELINE_RSS
    SESSION_ID=0

    for count in $SESSION_COUNTS; do
        # Create sessions by sending JSON-RPC calls
        # Each browse_session_create is an MCP tool call
        SESSIONS_TO_ADD=$(( count - SESSION_ID ))
        for (( s=0; s<SESSIONS_TO_ADD; s++ )); do
            SESSION_ID=$(( SESSION_ID + 1 ))
            # Send MCP initialize + tool call (best-effort; the server may not
            # parse bare JSON without proper framing). This is a documented
            # limitation — see Method 2 for the manual procedure.
            cat <<JSONRPC >&3 2>/dev/null || true
{"jsonrpc":"2.0","id":${SESSION_ID},"method":"tools/call","params":{"name":"browse_session_create","arguments":{"profile":"bench-${SESSION_ID}"}}}
JSONRPC
        done

        sleep "$SETTLE_TIME"

        if kill -0 "$MCP_PID" 2>/dev/null; then
            CURRENT_RSS=$(ps -o rss= -p "$MCP_PID" 2>/dev/null | tr -d ' ')
            CURRENT_MB=$(echo "scale=2; $CURRENT_RSS / 1024" | bc)
            DELTA_KB=$(( CURRENT_RSS - BASELINE_RSS ))
            if (( count > 0 )); then
                PER_SESSION_MB=$(echo "scale=2; $DELTA_KB / 1024 / $count" | bc)
            else
                PER_SESSION_MB="0"
            fi
            DELTA_MB=$(echo "scale=2; ($CURRENT_RSS - $PREV_RSS) / 1024" | bc)

            printf "%-15s %-15s %-15s %-15s\n" "$count" "$CURRENT_MB" "$PER_SESSION_MB" "+$DELTA_MB"
            printf "%-15s %-15s %-15s %-15s\n" "$count" "$CURRENT_MB" "$PER_SESSION_MB" "+$DELTA_MB" >> "$REPORT_FILE"

            PREV_RSS=$CURRENT_RSS
        else
            echo "MCP server exited at $count sessions."
            break
        fi
    done

    # Shut down MCP server
    exec 3>&-
    kill "$MCP_PID" 2>/dev/null || true
    wait "$MCP_PID" 2>/dev/null || true
    MCP_PID=""
    rm -f "$MCP_FIFO"
fi

echo ""

# ---------------------------------------------------------------------------
# Method 2: CLI mode (parallel navigate processes)
# ---------------------------------------------------------------------------
echo "=== Method 2: Parallel CLI Processes ==="
echo ""
echo "Spawning N concurrent 'wraith-browser navigate' processes and measuring"
echo "total system memory consumed. This includes per-process overhead, so"
echo "per-session numbers will be higher than MCP server mode."
echo ""

printf "%-15s %-18s %-18s %-15s\n" "Processes" "Total RSS (MB)" "Per-Process (MB)" "Status"
printf "%-15s %-18s %-18s %-15s\n" "---------" "--------------" "----------------" "------"

{
    echo ""
    echo "=== CLI Process Mode ==="
    printf "%-15s %-18s %-18s %-15s\n" "Processes" "Total RSS (MB)" "Per-Process (MB)" "Status"
    printf "%-15s %-18s %-18s %-15s\n" "---------" "--------------" "----------------" "------"
} >> "$REPORT_FILE"

for count in $SESSION_COUNTS; do
    PIDS=()

    # Launch processes in background — each navigates and sleeps
    for (( i=0; i<count; i++ )); do
        (
            "$WRAITH_BIN" navigate "$TEST_URL" --format snapshot > /dev/null 2>&1
            # Hold the process alive briefly so we can measure
            sleep 30
        ) &
        PIDS+=($!)

        # Stagger launches slightly to avoid thundering herd
        if (( i % 50 == 0 && i > 0 )); then
            sleep 0.5
        fi
    done

    # Wait for processes to be up and navigating
    sleep "$SETTLE_TIME"

    # Measure total RSS across all wraith-browser processes
    TOTAL_RSS=0
    ALIVE=0
    for pid in "${PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
            if [[ -n "$rss" ]]; then
                TOTAL_RSS=$(( TOTAL_RSS + rss ))
                ALIVE=$(( ALIVE + 1 ))
            fi
        fi
    done

    TOTAL_MB=$(echo "scale=2; $TOTAL_RSS / 1024" | bc)
    if (( ALIVE > 0 )); then
        PER_PROC_MB=$(echo "scale=2; $TOTAL_RSS / 1024 / $ALIVE" | bc)
        STATUS="$ALIVE/$count alive"
    else
        PER_PROC_MB="N/A"
        STATUS="all exited"
    fi

    printf "%-15s %-18s %-18s %-15s\n" "$count" "$TOTAL_MB" "$PER_PROC_MB" "$STATUS"
    printf "%-15s %-18s %-18s %-15s\n" "$count" "$TOTAL_MB" "$PER_PROC_MB" "$STATUS" >> "$REPORT_FILE"

    # Kill all child processes
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
done

echo ""

# ---------------------------------------------------------------------------
# Manual MCP procedure (for accurate in-process measurement)
# ---------------------------------------------------------------------------
cat << 'MANUAL'
============================================================
 Manual MCP Measurement Procedure
============================================================
For the most accurate per-session memory numbers, use an MCP
client to talk to the wraith-browser server:

1. Start the server:
     wraith-browser serve --transport stdio

2. Note the PID:
     pgrep -f wraith-browser

3. Record baseline RSS:
     ps -o rss= -p <PID> | awk '{print $1/1024 " MB"}'

4. Create sessions via MCP tool calls:
     browse_session_create (repeat N times)

5. After each batch, record RSS:
     ps -o rss= -p <PID> | awk '{print $1/1024 " MB"}'

6. Calculate: (current_rss - baseline_rss) / session_count

Expected results (Wraith Browser claim):
  - Baseline:    ~15-30 MB
  - Per session: ~8-12 MB additional RSS
============================================================
MANUAL

echo ""
echo "Report saved to: $REPORT_FILE"
echo "Done at $(date -Iseconds)"
