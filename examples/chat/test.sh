#!/usr/bin/env bash
set -e

# Integration test for the chat CLI
# Exercises: create, send (multi-user), messages, messages --json
#
# Set CHAT_BIN to a pre-built binary to skip cargo build.
# Otherwise falls back to `cargo run`.

DATA_DIR=$(mktemp -d)
if [ -n "${CHAT_BIN:-}" ]; then
    CMD="$CHAT_BIN --data-dir $DATA_DIR"
else
    CMD="cargo run --quiet -p example-chat -- --data-dir $DATA_DIR"
fi
PASS=0
FAIL=0

cleanup() {
    rm -rf "$DATA_DIR"
}
trap cleanup EXIT

assert_contains() {
    local label="$1" output="$2" expected="$3"
    if echo "$output" | grep -qF "$expected"; then
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label"
        echo "    expected to contain: $expected"
        echo "    got: $output"
        FAIL=$((FAIL + 1))
    fi
}

assert_count() {
    local label="$1" output="$2" expected="$3"
    local count
    count=$(echo "$output" | grep -c . || true)
    if [ "$count" -eq "$expected" ]; then
        echo "  PASS: $label (got $count lines)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label"
        echo "    expected $expected lines, got $count"
        echo "    output: $output"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== Eidetica Chat Integration Test ==="
echo "Data directory: $DATA_DIR"
echo

# ── Create a room ──────────────────────────────────────────────
echo "--- Create room ---"
TICKET=$($CMD --username alice create 2>/dev/null)
assert_contains "ticket starts with eidetica:" "$TICKET" "eidetica:?db="
echo

# ── Send messages ──────────────────────────────────────────────
echo "--- Send messages ---"
OUT=$($CMD --username alice send "$TICKET" "hello world" 2>&1)
assert_contains "alice send succeeds" "$OUT" "Message sent"

OUT=$($CMD --username bob send "$TICKET" "hey alice!" 2>&1)
assert_contains "bob send succeeds" "$OUT" "Message sent"

OUT=$($CMD --username alice send "$TICKET" "welcome bob" 2>&1)
assert_contains "alice second send succeeds" "$OUT" "Message sent"
echo

# ── Read messages ──────────────────────────────────────────────
echo "--- Read messages ---"
MSGS=$($CMD --username reader messages "$TICKET" 2>/dev/null)
assert_count "3 messages total" "$MSGS" 3
assert_contains "alice's first message" "$MSGS" "alice: hello world"
assert_contains "bob's message" "$MSGS" "bob: hey alice!"
assert_contains "alice's second message" "$MSGS" "alice: welcome bob"
echo

# ── Read messages with --limit ─────────────────────────────────
echo "--- Read messages with --limit ---"
MSGS=$($CMD --username reader messages "$TICKET" -n 2 2>/dev/null)
assert_count "limit=2 returns 2 messages" "$MSGS" 2
assert_contains "limit shows latest: bob" "$MSGS" "bob: hey alice!"
assert_contains "limit shows latest: alice" "$MSGS" "alice: welcome bob"
echo

# ── Read messages as JSON ──────────────────────────────────────
echo "--- Read messages as JSON ---"
JSON=$($CMD --username reader messages "$TICKET" --json 2>/dev/null)
assert_count "3 JSON lines" "$JSON" 3
assert_contains "JSON has author field" "$JSON" '"author":'
assert_contains "JSON has content field" "$JSON" '"content":'
assert_contains "JSON has timestamp field" "$JSON" '"timestamp":'
echo

# ── Persistence: re-read after new process ─────────────────────
echo "--- Persistence check ---"
MSGS2=$($CMD --username reader2 messages "$TICKET" 2>/dev/null)
assert_count "new reader sees all 3 messages" "$MSGS2" 3
echo

# ── DB file exists ─────────────────────────────────────────────
echo "--- Storage ---"
if [ -f "$DATA_DIR/chat.db" ]; then
    echo "  PASS: chat.db exists"
    PASS=$((PASS + 1))
else
    echo "  FAIL: chat.db not found in $DATA_DIR"
    FAIL=$((FAIL + 1))
fi
echo

# ── Encrypted room ─────────────────────────────────────────────
ENC_DIR=$(mktemp -d)
if [ -n "${CHAT_BIN:-}" ]; then
    ECMD="$CHAT_BIN --data-dir $ENC_DIR"
else
    ECMD="cargo run --quiet -p example-chat -- --data-dir $ENC_DIR"
fi

echo "--- Encrypted room ---"
ENC_TICKET=$($ECMD --username alice create --password s3cret 2>/dev/null)
assert_contains "encrypted ticket" "$ENC_TICKET" "eidetica:?db="

$ECMD --username alice send "$ENC_TICKET" "secret message" --password s3cret 2>&1 | grep -q "Message sent"
echo "  PASS: send to encrypted room"
PASS=$((PASS + 1))

ENC_MSGS=$($ECMD --username reader messages "$ENC_TICKET" --password s3cret 2>/dev/null)
assert_contains "read encrypted messages" "$ENC_MSGS" "secret message"

NO_PW=$($ECMD --username reader messages "$ENC_TICKET" 2>/dev/null)
if [ -z "$NO_PW" ]; then
    echo "  PASS: no messages without password"
    PASS=$((PASS + 1))
else
    echo "  FAIL: messages visible without password"
    FAIL=$((FAIL + 1))
fi

rm -rf "$ENC_DIR"
echo

# ── Summary ────────────────────────────────────────────────────
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
