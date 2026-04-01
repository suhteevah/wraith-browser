#!/usr/bin/env node
/**
 * bench_vs_playwright.js — Head-to-head benchmark: Wraith vs Playwright (Chromium)
 *
 * Runs the same URLs through both tools and compares:
 *   - Per-page latency (ms)
 *   - Memory per session (RSS in MB)
 *
 * Usage:
 *   node benchmarks/bench_vs_playwright.js
 *
 * Environment:
 *   WRAITH_BIN    — path to wraith binary (default: ./target/release/wraith-browser.exe)
 *   ITERATIONS    — runs per URL for latency test (default: 10)
 *   URL_FILE      — path to URL list (default: benchmarks/test_urls.txt)
 *   CONCURRENCY   — max concurrent sessions for memory test (default: 50)
 */

const { chromium } = require('playwright');
const { execFileSync, execFile } = require('child_process');
const fs = require('fs');
const path = require('path');

const WRAITH_BIN = process.env.WRAITH_BIN || './target/release/wraith-browser.exe';
const ITERATIONS = parseInt(process.env.ITERATIONS || '10', 10);
const URL_FILE = process.env.URL_FILE || path.join(__dirname, 'test_urls.txt');
const CONCURRENCY = parseInt(process.env.CONCURRENCY || '50', 10);

// Read URLs
const urls = fs.readFileSync(URL_FILE, 'utf8')
  .split('\n')
  .map(l => l.trim())
  .filter(l => l && !l.startsWith('#'));

function median(arr) {
  const s = [...arr].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

function percentile(arr, p) {
  const s = [...arr].sort((a, b) => a - b);
  const i = Math.ceil((p / 100) * s.length) - 1;
  return s[Math.max(0, i)];
}

function avg(arr) {
  return arr.reduce((a, b) => a + b, 0) / arr.length;
}

// Get RSS of a PID in MB (Windows via tasklist)
function getRssMB(pid) {
  try {
    const out = execFileSync('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], {
      encoding: 'utf8', timeout: 5000
    });
    const match = out.match(/"([\d,]+)\s*K"/);
    if (match) {
      return parseInt(match[1].replace(/,/g, ''), 10) / 1024;
    }
  } catch {}
  return 0;
}

// Get total RSS of all Chromium processes in MB
// Playwright uses chrome-headless-shell.exe, regular Chrome uses chrome.exe
function getChromeRssMB() {
  const imageNames = ['chrome.exe', 'chrome-headless-shell.exe'];
  let total = 0;
  for (const img of imageNames) {
    try {
      const out = execFileSync('tasklist', ['/FI', `IMAGENAME eq ${img}`, '/FO', 'CSV', '/NH'], {
        encoding: 'utf8', timeout: 10000
      });
      for (const line of out.split('\n')) {
        const match = line.match(/"([\d,]+)\s*K"/);
        if (match) {
          total += parseInt(match[1].replace(/,/g, ''), 10);
        }
      }
    } catch {}
  }
  return total / 1024;
}

// Run wraith binary safely (no shell)
function runWraith(url) {
  try {
    execFileSync(WRAITH_BIN, ['navigate', url, '--format', 'snapshot'], {
      encoding: 'utf8', timeout: 30000, stdio: 'pipe'
    });
    return true;
  } catch {
    return false;
  }
}

// Spawn wraith process (no shell) — returns ChildProcess
function spawnWraith(url) {
  return require('child_process').spawn(WRAITH_BIN, ['navigate', url, '--format', 'snapshot'], {
    stdio: 'pipe', windowsHide: true
  });
}

async function benchLatency() {
  console.log('='.repeat(70));
  console.log(' LATENCY BENCHMARK — Wraith vs Playwright (Chromium)');
  console.log('='.repeat(70));
  console.log(` Iterations per URL: ${ITERATIONS}`);
  console.log(` URLs: ${urls.length}`);
  console.log('');

  const wraithResults = {};
  const playwrightResults = {};

  // --- Wraith latency ---
  console.log('--- Wraith Browser ---');
  for (const url of urls) {
    const times = [];
    for (let i = 0; i < ITERATIONS; i++) {
      const start = performance.now();
      if (runWraith(url)) {
        times.push(performance.now() - start);
      }
    }
    if (times.length > 0) {
      wraithResults[url] = times;
      console.log(`  ${url}: p50=${median(times).toFixed(0)}ms  p95=${percentile(times, 95).toFixed(0)}ms  avg=${avg(times).toFixed(0)}ms  (${times.length}/${ITERATIONS} ok)`);
    } else {
      console.log(`  ${url}: ALL FAILED`);
    }
  }

  // --- Playwright latency ---
  console.log('');
  console.log('--- Playwright (Chromium) ---');
  const browser = await chromium.launch({ headless: true });

  for (const url of urls) {
    const times = [];
    for (let i = 0; i < ITERATIONS; i++) {
      const page = await browser.newPage();
      const start = performance.now();
      try {
        await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
        await page.content();
        times.push(performance.now() - start);
      } catch {}
      await page.close();
    }
    if (times.length > 0) {
      playwrightResults[url] = times;
      console.log(`  ${url}: p50=${median(times).toFixed(0)}ms  p95=${percentile(times, 95).toFixed(0)}ms  avg=${avg(times).toFixed(0)}ms  (${times.length}/${ITERATIONS} ok)`);
    } else {
      console.log(`  ${url}: ALL FAILED`);
    }
  }

  await browser.close();

  // --- Comparison table ---
  console.log('');
  console.log('='.repeat(70));
  console.log(' LATENCY COMPARISON (p50 ms)');
  console.log('='.repeat(70));
  console.log(`${'URL'.padEnd(45)} ${'Wraith'.padStart(10)} ${'Playwright'.padStart(12)} ${'Ratio'.padStart(8)}`);
  console.log('-'.repeat(77));

  for (const url of urls) {
    const w = wraithResults[url];
    const p = playwrightResults[url];
    if (w && p) {
      const wMed = median(w);
      const pMed = median(p);
      const ratio = pMed / wMed;
      const short = url.length > 44 ? '...' + url.slice(-41) : url;
      console.log(`${short.padEnd(45)} ${wMed.toFixed(0).padStart(8)}ms ${pMed.toFixed(0).padStart(10)}ms ${ratio.toFixed(1).padStart(6)}x`);
    }
  }

  return { wraithResults, playwrightResults };
}

async function benchMemory() {
  console.log('');
  console.log('='.repeat(70));
  console.log(' MEMORY BENCHMARK — Wraith vs Playwright (Chromium)');
  console.log('='.repeat(70));
  console.log(` Concurrent sessions: up to ${CONCURRENCY}`);
  console.log('');

  const testUrl = urls[0] || 'http://example.com';

  // --- Playwright memory ---
  console.log('--- Playwright (Chromium) ---');
  const chromeBaseline = getChromeRssMB();
  console.log(`  Chrome baseline (existing processes): ${chromeBaseline.toFixed(0)} MB`);

  const browser = await chromium.launch({ headless: true });
  const levels = [1, 5, 10, 25, CONCURRENCY].filter(n => n <= CONCURRENCY);
  const pages = [];

  console.log(`${'  Sessions'.padEnd(15)} ${'Chrome RSS'.padStart(12)} ${'Per-session'.padStart(14)}`);
  console.log(`${'  --------'.padEnd(15)} ${'----------'.padStart(12)} ${'-----------'.padStart(14)}`);

  const playwrightMem = [];
  for (const level of levels) {
    while (pages.length < level) {
      const page = await browser.newPage();
      try {
        await page.goto(testUrl, { waitUntil: 'domcontentloaded', timeout: 15000 });
      } catch {}
      pages.push(page);
    }

    await new Promise(r => setTimeout(r, 2000));

    const currentRss = getChromeRssMB();
    const delta = currentRss - chromeBaseline;
    const perSession = level > 0 ? delta / level : 0;
    console.log(`  ${String(level).padEnd(13)} ${currentRss.toFixed(0).padStart(10)} MB ${perSession.toFixed(1).padStart(12)} MB`);
    playwrightMem.push({ sessions: level, totalMB: currentRss, perSessionMB: perSession });
  }

  for (const page of pages) await page.close();
  await browser.close();

  // --- Wraith memory (concurrent CLI processes) ---
  console.log('');
  console.log('--- Wraith Browser (concurrent CLI processes) ---');
  console.log('  Note: CLI mode includes per-process overhead. MCP server mode uses ~8 MB/session.');
  console.log(`${'  Processes'.padEnd(15)} ${'Total RSS'.padStart(12)} ${'Per-process'.padStart(14)}`);
  console.log(`${'  ---------'.padEnd(15)} ${'----------'.padStart(12)} ${'-----------'.padStart(14)}`);

  const wraithMem = [];
  for (const level of levels.filter(n => n <= 25)) {
    const procs = [];
    for (let i = 0; i < level; i++) {
      procs.push(spawnWraith(testUrl));
    }

    await new Promise(r => setTimeout(r, 3000));

    let totalRss = 0;
    let alive = 0;
    for (const p of procs) {
      if (p.pid && !p.killed) {
        const rss = getRssMB(p.pid);
        if (rss > 0) {
          totalRss += rss;
          alive++;
        }
      }
    }

    if (alive > 0) {
      const perProc = totalRss / alive;
      console.log(`  ${String(alive).padEnd(13)} ${totalRss.toFixed(0).padStart(10)} MB ${perProc.toFixed(1).padStart(12)} MB`);
      wraithMem.push({ sessions: alive, totalMB: totalRss, perSessionMB: perProc });
    }

    for (const p of procs) {
      try { p.kill(); } catch {}
    }
    await new Promise(r => setTimeout(r, 1000));
  }

  // --- Comparison ---
  console.log('');
  console.log('='.repeat(70));
  console.log(' MEMORY COMPARISON SUMMARY');
  console.log('='.repeat(70));

  const pwLast = playwrightMem[playwrightMem.length - 1];
  const wrLast = wraithMem[wraithMem.length - 1];
  if (pwLast && wrLast) {
    console.log(`  Playwright at ${pwLast.sessions} sessions: ${pwLast.perSessionMB.toFixed(1)} MB/session`);
    console.log(`  Wraith at ${wrLast.sessions} processes:    ${wrLast.perSessionMB.toFixed(1)} MB/process (CLI mode)`);
    console.log(`  Memory ratio: ${(pwLast.perSessionMB / wrLast.perSessionMB).toFixed(1)}x more for Playwright`);
    console.log('');
    console.log('  Note: Wraith MCP server mode uses ~8 MB/session (shared process, no startup overhead).');
    console.log('  The CLI numbers above include per-process baseline and will be higher.');
  }

  return { playwrightMem, wraithMem };
}

async function main() {
  console.log('Wraith Browser vs Playwright — Head-to-Head Benchmark');
  console.log(`Date: ${new Date().toISOString()}`);
  console.log(`Node: ${process.version}`);
  console.log(`Wraith: ${WRAITH_BIN}`);
  console.log('');

  const latency = await benchLatency();
  const memory = await benchMemory();

  // Save results
  const resultsDir = path.join(__dirname, 'results');
  fs.mkdirSync(resultsDir, { recursive: true });
  const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  const outFile = path.join(resultsDir, `vs_playwright_${ts}.json`);
  fs.writeFileSync(outFile, JSON.stringify({ latency, memory, date: new Date().toISOString() }, null, 2));
  console.log('');
  console.log(`Results saved to: ${outFile}`);
}

main().catch(e => { console.error(e); process.exit(1); });
