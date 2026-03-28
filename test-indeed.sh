#!/bin/bash
# Test Firefox 136 bypass against Indeed.com
# Target: Jobs near 95926 (Tulare, CA) paying $25+/hr

BINARY="./target/release/wraith-browser.exe"

if [ ! -f "$BINARY" ]; then
    echo "Release binary not found, trying debug..."
    BINARY="./target/debug/wraith-browser.exe"
fi

if [ ! -f "$BINARY" ]; then
    echo "No binary found. Run: cargo build --release --features stealth-tls"
    exit 1
fi

echo "=== Testing Firefox 136 TLS bypass against Indeed.com ==="
echo "Binary: $BINARY"
echo ""

# Indeed search URL for jobs near 95926, $25+/hr
# Indeed URL format: /jobs?q=&l=95926&sc=0kf%3Aattr(DSQF7)%3B&fromage=14
URL="https://www.indeed.com/jobs?q=&l=95926&sc=0kf%3Aattr(DSQF7)%3B&fromage=14&salary=%2425%2B"

echo "URL: $URL"
echo ""
echo "--- Attempting navigation ---"
$BINARY navigate "$URL" 2>&1

echo ""
echo "--- Attempting extract ---"
$BINARY extract "$URL" --max-tokens 4000 2>&1
