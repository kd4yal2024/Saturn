#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

const EXPECTED_NORMAL = ['lan-48k', 'lan-96k', 'lan-192k', 'vpn-48k', 'vpn-96k', 'vpn-192k'];
const EXPECTED_SLOW = ['lan-192k-slow-client', 'vpn-192k-slow-client'];
const SAFETY_P99_LIMIT_US = 5_000;
const CONTROL_P99_LIMIT_US = 20_000;
const IQ_RATE_BASELINE_TOLERANCE = 0.02;

function usage() {
  return [
    'Usage: saturn-remote-phase37-analyze.mjs [capture-dir] [--out report.md] [--strict]',
    '',
    'Capture dir defaults to ~/Documents/perf-captures/phase36.',
    'The analyzer reads Phase 36 host .log files plus browser JSON summaries/samples.',
  ].join('\n');
}

function expandHome(filePath) {
  if (!filePath || !filePath.startsWith('~')) return filePath;
  const home = process.env.HOME || '/home/pi';
  return path.join(home, filePath.slice(1));
}

function parseArgs(argv) {
  const args = argv.slice(2);
  let captureDir = '~/Documents/perf-captures/phase36';
  let outFile = null;
  let strict = false;
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === '--help' || arg === '-h') {
      console.log(usage());
      process.exit(0);
    }
    if (arg === '--out') {
      outFile = args[i + 1] || null;
      i += 1;
      continue;
    }
    if (arg === '--strict') {
      strict = true;
      continue;
    }
    if (!arg.startsWith('--')) {
      captureDir = arg;
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  return {
    captureDir: expandHome(captureDir),
    outFile: outFile ? expandHome(outFile) : null,
    strict,
  };
}

function walkFiles(dir) {
  const files = [];
  if (!fs.existsSync(dir)) return files;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...walkFiles(fullPath));
    } else if (entry.isFile()) {
      files.push(fullPath);
    }
  }
  return files.sort();
}

function inferLabelFromName(filePath) {
  const name = path.basename(filePath).toLowerCase();
  const slow = name.includes('slow') ? '-slow-client' : '';
  const network = name.includes('vpn') ? 'vpn' : name.includes('lan') ? 'lan' : null;
  const rateMatch = name.match(/(?:^|[^0-9])(48|96|192)\s*k/);
  if (network && rateMatch) return `${network}-${rateMatch[1]}k${slow}`;
  return null;
}

function numeric(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function parseHostLog(filePath) {
  const text = fs.readFileSync(filePath, 'utf8');
  if (!text.includes('Saturn Remote Phase 36 host capture')) return null;
  const label = text.match(/^session_label=(.+)$/m)?.[1]?.trim() || inferLabelFromName(filePath) || path.basename(filePath);
  const durationSec = numeric(text.match(/^duration_sec=(.+)$/m)?.[1], 0);
  const samples = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.includes('saturn-bridge: diag ')) continue;
    const fields = {};
    for (const match of line.matchAll(/([a-zA-Z0-9_]+)=([^\s]+)/g)) {
      fields[match[1]] = numeric(match[2], Number.NaN);
    }
    samples.push(fields);
  }
  const ddcRates = samples.map((sample) => sample.ddc_s).filter(Number.isFinite);
  const maxOf = (key) => {
    const values = samples.map((sample) => sample[key]).filter(Number.isFinite);
    return values.length ? Math.max(...values) : 0;
  };
  const avg = (values) => values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : 0;
  return {
    file: filePath,
    label,
    durationSec,
    diagSamples: samples.length,
    avgDdcPerSec: Number(avg(ddcRates).toFixed(2)),
    maxDdcPerSec: ddcRates.length ? Math.max(...ddcRates) : 0,
    maxSafetyP99Us: maxOf('safety_p99_us'),
    maxControlP99Us: maxOf('control_p99_us'),
    totalDisplayReplaced: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.display_replaced_s) ? sample.display_replaced_s : 0), 0),
    totalDisplayDropped: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.display_dropped_s) ? sample.display_dropped_s : 0), 0),
    totalDisplayRateLimited: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.display_rate_limited_s) ? sample.display_rate_limited_s : 0), 0),
    totalAudioDropped: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.audio_dropped_s) ? sample.audio_dropped_s : 0), 0),
    totalSendBlockedMs: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.send_blocked_ms) ? sample.send_blocked_ms : 0), 0),
    maxOutboundHighWatermarkBytes: maxOf('out_hwm_bytes'),
    maxTcpOutqHighWatermarkBytes: maxOf('tcp_outq_hwm_bytes'),
    totalSafetyQueueDepthOverflow: samples.reduce((sum, sample) => sum + (Number.isFinite(sample.safety_depth_overflow) ? sample.safety_depth_overflow : 0), 0),
  };
}

function summarizeSamples(samples, filePath) {
  const safeSamples = Array.isArray(samples) ? samples : [];
  const frameRates = safeSamples.map((sample) => numeric(sample.frameRate)).filter(Number.isFinite);
  const last = safeSamples[safeSamples.length - 1] || {};
  const maxOf = (key) => {
    const values = safeSamples.map((sample) => numeric(sample[key], Number.NaN)).filter(Number.isFinite);
    return values.length ? Math.max(...values) : 0;
  };
  const sumOf = (key) => safeSamples.reduce((sum, sample) => sum + numeric(sample[key]), 0);
  return {
    file: filePath,
    sampleCount: safeSamples.length,
    startedAt: safeSamples[0]?.wallTime || null,
    endedAt: last.wallTime || null,
    avgFrameRate: frameRates.length
      ? Number((frameRates.reduce((sum, value) => sum + value, 0) / frameRates.length).toFixed(2))
      : 0,
    minFrameRate: frameRates.length ? Math.min(...frameRates) : 0,
    maxFrameRate: frameRates.length ? Math.max(...frameRates) : 0,
    maxIqIdleMs: maxOf('iqIdleMs'),
    maxBridgeRttMs: maxOf('bridgeRttMs'),
    maxBackpressureSafetyP99Us: maxOf('backpressureSafetyP99Us'),
    maxBackpressureControlP99Us: maxOf('backpressureControlP99Us'),
    totalDisplayReplaced: sumOf('displayReplacedPerSec'),
    totalDisplayDropped: sumOf('displayDroppedPerSec'),
    totalBridgeAudioDropped: sumOf('bridgeAudioDroppedPerSec'),
    finalBridgeAudioSeqGapCount: numeric(last.bridgeAudioSeqGapCount),
    finalAudioSeqGapCount: numeric(last.audioSeqGapCount),
    totalAudioPanicDrainCount: sumOf('audioPanicDrainCount'),
    totalSendBlockedMs: sumOf('sendBlockedMs'),
    maxOutboundHighWatermarkBytes: maxOf('outboundHighWatermarkBytes'),
    totalSafetyQueueDepthOverflowCount: sumOf('safetyQueueDepthOverflowCount'),
  };
}

function parseBrowserJson(filePath) {
  if (!filePath.endsWith('.json')) return null;
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch {
    return null;
  }

  const summary = Array.isArray(parsed)
    ? summarizeSamples(parsed, filePath)
    : parsed && typeof parsed === 'object' && 'sampleCount' in parsed
      ? { ...parsed, file: filePath }
      : null;
  if (!summary) return null;
  return {
    ...summary,
    label: inferLabelFromName(filePath) || summary.sessionLabel || path.basename(filePath, '.json'),
  };
}

function mergeSessions(hostLogs, browserSummaries) {
  const sessions = new Map();
  const ensure = (label) => {
    if (!sessions.has(label)) sessions.set(label, { label, host: [], browser: [] });
    return sessions.get(label);
  };
  for (const host of hostLogs) ensure(host.label).host.push(host);
  for (const browser of browserSummaries) ensure(browser.label).browser.push(browser);
  return [...sessions.values()].sort((a, b) => a.label.localeCompare(b.label));
}

function sessionBrowserMax(session, key) {
  const values = session.browser.map((item) => numeric(item[key], Number.NaN)).filter(Number.isFinite);
  return values.length ? Math.max(...values) : 0;
}

function sessionHostMax(session, key) {
  const values = session.host.map((item) => numeric(item[key], Number.NaN)).filter(Number.isFinite);
  return values.length ? Math.max(...values) : 0;
}

function evaluateSession(session) {
  const safetyP99 = Math.max(
    sessionBrowserMax(session, 'maxBackpressureSafetyP99Us'),
    sessionHostMax(session, 'maxSafetyP99Us'),
  );
  const controlP99 = Math.max(
    sessionBrowserMax(session, 'maxBackpressureControlP99Us'),
    sessionHostMax(session, 'maxControlP99Us'),
  );
  const safetyOverflow = Math.max(
    sessionBrowserMax(session, 'totalSafetyQueueDepthOverflowCount'),
    sessionHostMax(session, 'totalSafetyQueueDepthOverflow'),
  );
  const failures = [];
  const warnings = [];
  if (safetyP99 > SAFETY_P99_LIMIT_US) failures.push(`safety p99 ${safetyP99} us > ${SAFETY_P99_LIMIT_US} us`);
  if (controlP99 > CONTROL_P99_LIMIT_US) failures.push(`control p99 ${controlP99} us > ${CONTROL_P99_LIMIT_US} us`);
  if (safetyOverflow > 0) failures.push(`safety queue overflow count ${safetyOverflow}`);
  if (!session.browser.length) warnings.push('missing browser summary');
  if (!session.host.length) warnings.push('missing host capture');
  if (sessionBrowserMax(session, 'totalAudioPanicDrainCount') > 0) warnings.push('audio panic drain observed');
  if (sessionBrowserMax(session, 'finalAudioSeqGapCount') > 0) warnings.push('browser audio sequence gap observed');
  return { safetyP99, controlP99, safetyOverflow, failures, warnings };
}

function evaluateIqRateIsolation(sessions) {
  const byLabel = new Map(sessions.map((session) => [session.label, session]));
  const checks = [];
  for (const slowLabel of EXPECTED_SLOW) {
    const baselineLabel = slowLabel.replace('-slow-client', '');
    const baseline = byLabel.get(baselineLabel);
    const slow = byLabel.get(slowLabel);
    if (!baseline || !slow || !baseline.host.length || !slow.host.length) {
      checks.push({
        label: slowLabel,
        baselineLabel,
        status: 'missing',
        message: 'missing host baseline or slow-client host capture',
      });
      continue;
    }
    const baselineRate = Math.max(...baseline.host.map((host) => host.avgDdcPerSec));
    const slowRate = Math.max(...slow.host.map((host) => host.avgDdcPerSec));
    const allowedDrop = baselineRate * IQ_RATE_BASELINE_TOLERANCE;
    const delta = baselineRate - slowRate;
    checks.push({
      label: slowLabel,
      baselineLabel,
      baselineRate,
      slowRate,
      delta,
      status: Math.abs(delta) <= allowedDrop ? 'pass' : 'fail',
      message: `baseline ${baselineRate.toFixed(2)} ddc/s, slow ${slowRate.toFixed(2)} ddc/s, delta ${delta.toFixed(2)}`,
    });
  }
  return checks;
}

function formatTable(rows, headers) {
  const line = `| ${headers.join(' | ')} |`;
  const sep = `| ${headers.map(() => '---').join(' | ')} |`;
  const body = rows.map((row) => `| ${headers.map((header) => `${row[header] ?? ''}`).join(' | ')} |`);
  return [line, sep, ...body].join('\n');
}

function buildReport({ captureDir, files, hostLogs, browserSummaries, sessions }) {
  const expected = [...EXPECTED_NORMAL, ...EXPECTED_SLOW];
  const presentLabels = new Set(sessions.map((session) => session.label));
  const missing = expected.filter((label) => !presentLabels.has(label));
  const evaluations = sessions.map((session) => ({ session, result: evaluateSession(session) }));
  const failures = evaluations.flatMap(({ session, result }) =>
    result.failures.map((failure) => `${session.label}: ${failure}`),
  );
  const warnings = evaluations.flatMap(({ session, result }) =>
    result.warnings.map((warning) => `${session.label}: ${warning}`),
  );
  const iqChecks = evaluateIqRateIsolation(sessions);
  for (const check of iqChecks) {
    if (check.status === 'fail') failures.push(`${check.label}: IQ rate isolation failed (${check.message})`);
    if (check.status === 'missing') warnings.push(`${check.label}: ${check.message}`);
  }

  const rows = sessions.map((session) => {
    const result = evaluateSession(session);
    return {
      Session: session.label,
      Browser: session.browser.length,
      Host: session.host.length,
      'Safety p99 us': result.safetyP99,
      'Control p99 us': result.controlP99,
      'Display repl': Math.max(
        sessionBrowserMax(session, 'totalDisplayReplaced'),
        sessionHostMax(session, 'totalDisplayReplaced'),
      ),
      'Display cap': sessionHostMax(session, 'totalDisplayRateLimited'),
      'Send blocked ms': Math.max(
        sessionBrowserMax(session, 'totalSendBlockedMs'),
        sessionHostMax(session, 'totalSendBlockedMs'),
      ),
      'Out HWM bytes': Math.max(
        sessionBrowserMax(session, 'maxOutboundHighWatermarkBytes'),
        sessionHostMax(session, 'maxOutboundHighWatermarkBytes'),
      ),
      'TCP outq HWM bytes': sessionHostMax(session, 'maxTcpOutqHighWatermarkBytes'),
      Status: result.failures.length ? 'fail' : result.warnings.length ? 'warn' : 'pass',
    };
  });

  const complete = missing.length === 0 && warnings.filter((warning) => warning.includes('missing')).length === 0;
  const readyForAutoRate = complete && failures.length === 0;

  return [
    '# Saturn Remote Phase 37 Analysis',
    '',
    `Generated: ${new Date().toISOString()}`,
    `Capture directory: ${captureDir}`,
    `Files scanned: ${files.length}`,
    `Host captures: ${hostLogs.length}`,
    `Browser summaries/samples: ${browserSummaries.length}`,
    '',
    '## Decision',
    '',
    readyForAutoRate
      ? 'Status: ready to derive Phase 37 adaptive-rate thresholds from the captured data.'
      : 'Status: not ready to implement adaptive-rate thresholds yet.',
    '',
    complete
      ? '- Matrix completeness: complete.'
      : `- Matrix completeness: incomplete. Missing expected labels: ${missing.length ? missing.join(', ') : 'none, but one or more sessions lack browser or host data'}.`,
    failures.length
      ? `- Failures: ${failures.length}.`
      : '- Failures: none found in available data.',
    warnings.length
      ? `- Warnings: ${warnings.length}.`
      : '- Warnings: none found in available data.',
    '',
    '## Session Summary',
    '',
    rows.length
      ? formatTable(rows, ['Session', 'Browser', 'Host', 'Safety p99 us', 'Control p99 us', 'Display repl', 'Display cap', 'Send blocked ms', 'Out HWM bytes', 'TCP outq HWM bytes', 'Status'])
      : 'No Phase 36 sessions found.',
    '',
    '## IQ Isolation Checks',
    '',
    iqChecks.length
      ? formatTable(
          iqChecks.map((check) => ({
            Session: check.label,
            Baseline: check.baselineLabel,
            Status: check.status,
            Detail: check.message,
          })),
          ['Session', 'Baseline', 'Status', 'Detail'],
        )
      : 'No slow-client checks were evaluated.',
    '',
    '## Failures',
    '',
    failures.length ? failures.map((failure) => `- ${failure}`).join('\n') : '- None.',
    '',
    '## Warnings',
    '',
    warnings.length ? warnings.map((warning) => `- ${warning}`).join('\n') : '- None.',
    '',
    '## Next Step',
    '',
    readyForAutoRate
      ? 'Use this report to set adaptive raw-IQ rate thresholds with hysteresis in Phase 37 implementation.'
      : 'Complete the Phase 36 LAN/VPN matrix, rerun this analyzer, and only then implement adaptive raw-IQ rate thresholds.',
    '',
  ].join('\n');
}

function main() {
  const { captureDir, outFile, strict } = parseArgs(process.argv);
  const files = walkFiles(captureDir);
  const hostLogs = files.map(parseHostLog).filter(Boolean);
  const browserSummaries = files.map(parseBrowserJson).filter(Boolean);
  const sessions = mergeSessions(hostLogs, browserSummaries);
  const report = buildReport({ captureDir, files, hostLogs, browserSummaries, sessions });
  if (outFile) {
    fs.mkdirSync(path.dirname(outFile), { recursive: true });
    fs.writeFileSync(outFile, report);
    console.log(`report_file=${outFile}`);
  } else {
    console.log(report);
  }
  if (strict && report.includes('Status: not ready')) process.exit(1);
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  console.error(usage());
  process.exit(2);
}
