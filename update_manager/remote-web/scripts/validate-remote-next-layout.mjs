#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const updateManagerRoot = resolve(remoteWebRoot, '..');
const templatePath = resolve(updateManagerRoot, 'templates/saturn-remote-next.html');
const outputRoot = join(tmpdir(), 'saturn-remote-next-layout');
const chromium = process.env.CHROMIUM || 'chromium';

const expectedPills = [
  'operator-conn-pill',
  'operator-owner-pill',
  'operator-rxtx-pill',
  'operator-rf-pill',
  'operator-transport-pill',
  'operator-latency-pill',
  'operator-audio-pill',
  'operator-fault-pill',
];

const scenarios = [
  { name: 'phone-portrait', width: 390, height: 844, layout: 'phone' },
  { name: 'phone-landscape', width: 844, height: 390, layout: 'phone' },
  { name: 'tablet', width: 1024, height: 768, layout: 'desktop' },
  { name: 'laptop', width: 1366, height: 768, layout: 'desktop' },
];

function requireFile(path, label) {
  if (!existsSync(path)) {
    throw new Error(`${label} does not exist: ${path}`);
  }
}

function validationScript(scenario) {
  return `<script>
(() => {
  const scenario = ${JSON.stringify(scenario)};
  const expectedPills = ${JSON.stringify(expectedPills)};
  function round(value) {
    return Math.round(value * 100) / 100;
  }
  function rectFor(element) {
    const rect = element.getBoundingClientRect();
    return {
      left: round(rect.left),
      top: round(rect.top),
      right: round(rect.right),
      bottom: round(rect.bottom),
      width: round(rect.width),
      height: round(rect.height)
    };
  }
  function visible(element) {
    const style = window.getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden" && Number(style.opacity) !== 0;
  }
  function overlap(a, b) {
    return a.left < b.right - 1 && a.right > b.left + 1 && a.top < b.bottom - 1 && a.bottom > b.top + 1;
  }
  function textOverflow(selector) {
    return Array.from(document.querySelectorAll(selector))
      .filter((element) => element.scrollWidth > element.clientWidth + 1 || element.scrollHeight > element.clientHeight + 1)
      .map((element) => ({
        id: element.id || element.closest(".operator-pill")?.id || "",
        text: element.textContent.trim(),
        clientWidth: element.clientWidth,
        scrollWidth: element.scrollWidth,
        clientHeight: element.clientHeight,
        scrollHeight: element.scrollHeight
      }));
  }
  function runValidation() {
    const strip = document.querySelector(".operator-state-strip");
    const pills = Array.from(document.querySelectorAll(".operator-pill"));
    const stripRect = strip ? rectFor(strip) : null;
    const boxes = pills.map((element) => ({ id: element.id, rect: rectFor(element), visible: visible(element) }));
    const missing = expectedPills.filter((id) => !document.getElementById(id));
    const invisible = boxes.filter((box) => !box.visible).map((box) => box.id);
    const geometryOverlaps = [];
    for (let i = 0; i < boxes.length; i += 1) {
      for (let j = i + 1; j < boxes.length; j += 1) {
        if (overlap(boxes[i].rect, boxes[j].rect)) {
          geometryOverlaps.push([boxes[i].id, boxes[j].id]);
        }
      }
    }
    const stripOverflow = stripRect
      ? boxes
          .filter((box) =>
            box.rect.left < stripRect.left - 1 ||
            box.rect.top < stripRect.top - 1 ||
            box.rect.right > stripRect.right + 1 ||
            box.rect.bottom > stripRect.bottom + 1
          )
          .map((box) => box.id)
      : expectedPills;
    const viewportOverflow = boxes
      .filter((box) => box.rect.left < -1 || box.rect.top < -1 || box.rect.right > window.innerWidth + 1)
      .map((box) => box.id);
    const layout = document.documentElement.dataset.layout || "";
    const failures = {
      missing,
      invisible,
      geometryOverlaps,
      stripOverflow,
      viewportOverflow,
      valueOverflow: textOverflow(".operator-pill-value"),
      layoutMismatch: layout === scenario.layout ? [] : [{ expected: scenario.layout, actual: layout }]
    };
    const ok = Object.values(failures).every((value) => Array.isArray(value) && value.length === 0);
    const report = {
      scenario,
      ok,
      layout,
      viewport: { width: window.innerWidth, height: window.innerHeight },
      stripRect,
      pills: boxes,
      failures,
      warnings: { labelOverflow: textOverflow(".operator-pill-label") }
    };
    const marker = document.createElement("script");
    marker.id = "saturn-layout-report";
    marker.type = "application/json";
    marker.textContent = JSON.stringify(report);
    document.body.appendChild(marker);
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", runValidation, { once: true });
  } else {
    runValidation();
  }
})();
</script>`;
}

function makeScenarioHtml(template, scenario) {
  const staticStateScript = `<script>
(() => {
  document.documentElement.dataset.layout = ${JSON.stringify(scenario.layout)};
  document.documentElement.dataset.phoneWaterfall = "hidden";
  const values = {
    "operator-conn": ["ok", "Connected", "Static validation state"],
    "operator-owner": ["warn", "Role pending", "Static validation state"],
    "operator-rxtx": ["keyed", "PTT ON AIR", "Static validation state"],
    "operator-rf": ["alarm", "RF disabled", "Static validation state"],
    "operator-transport": ["rx", "Tailnet", "Static validation state"],
    "operator-latency": ["warn", "888 ms RTT", "Static validation state"],
    "operator-audio": ["warn", "48 ms lead", "Static validation state"],
    "operator-fault": ["alarm", "Power trip", "Static validation state"]
  };
  for (const [prefix, [tone, value, title]] of Object.entries(values)) {
    const pill = document.getElementById(prefix + "-pill");
    const valueNode = document.getElementById(prefix + "-value");
    if (pill) {
      pill.dataset.tone = tone;
      pill.title = title;
    }
    if (valueNode) valueNode.textContent = value;
  }
})();
</script>`;

  const runtimeMarker = '  <!-- Runtime provided by SaturnRemoteNext bundle -->';
  const runtimeStart = template.indexOf(runtimeMarker);
  const bodyEnd = template.lastIndexOf('</body>');
  if (runtimeStart < 0 || bodyEnd < 0 || bodyEnd <= runtimeStart) {
    throw new Error('template does not contain the expected runtime marker and body end');
  }
  return (
    template
      .slice(0, runtimeStart)
      .replace('<html lang="en">', `<html lang="en" data-layout="${scenario.layout}" data-phone-waterfall="hidden">`)
      .replace(/  <link rel="preconnect" href="https:\/\/fonts\.googleapis\.com">\n/g, '')
      .replace(/  <link rel="preconnect" href="https:\/\/fonts\.gstatic\.com" crossorigin>\n/g, '')
      .replace(/  <link href="https:\/\/fonts\.googleapis\.com[^"]+" rel="stylesheet">\n/g, '') +
    `\n  <!-- Static state-strip validation harness. Runtime scripts intentionally removed. -->\n  ${staticStateScript}\n  ${validationScript(scenario)}\n` +
    template.slice(bodyEnd)
  );
}

function chromiumArgs(scenario, scenarioDir) {
  return [
    '--headless=new',
    '--disable-gpu',
    '--disable-dev-shm-usage',
    '--disable-extensions',
    '--disable-background-networking',
    '--disable-default-apps',
    '--disable-sync',
    '--hide-scrollbars',
    '--mute-audio',
    '--no-first-run',
    '--no-default-browser-check',
    '--no-sandbox',
    '--allow-file-access-from-files',
    '--force-device-scale-factor=1',
    `--user-data-dir=${join(scenarioDir, 'profile')}`,
    `--window-size=${scenario.width},${scenario.height}`,
    '--virtual-time-budget=2000',
  ];
}

function runChromium(args, label, timeoutMs = 75000) {
  return new Promise((resolveRun, rejectRun) => {
    const proc = spawn(chromium, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => {
      proc.kill('SIGTERM');
      rejectRun(new Error(`${label} timed out after ${timeoutMs} ms\n${stderr}`));
    }, timeoutMs);
    proc.stdout.on('data', (chunk) => {
      stdout += chunk.toString();
    });
    proc.stderr.on('data', (chunk) => {
      stderr += chunk.toString();
    });
    proc.once('error', (error) => {
      clearTimeout(timer);
      rejectRun(new Error(`${label} failed to start ${chromium}: ${error.message}`));
    });
    proc.once('exit', (code, signal) => {
      clearTimeout(timer);
      if (code === 0) {
        resolveRun({ stdout, stderr });
      } else {
        rejectRun(new Error(`${label} failed with ${signal || `exit ${code}`}\n${stderr || stdout}`));
      }
    });
  });
}

function extractReport(html) {
  const match = html.match(/<script id="saturn-layout-report" type="application\/json">([\s\S]*?)<\/script>/);
  if (!match) {
    throw new Error('Chromium DOM dump did not include saturn-layout-report marker');
  }
  return JSON.parse(match[1]);
}

async function validateScenario(scenario, pageUrl, scenarioDir) {
  const dump = await runChromium(
    [...chromiumArgs(scenario, scenarioDir), '--dump-dom', pageUrl],
    `DOM validation for ${scenario.name}`,
  );
  writeFileSync(join(scenarioDir, 'dump.html'), dump.stdout);
  const report = extractReport(dump.stdout);
  await runChromium(
    [...chromiumArgs(scenario, scenarioDir), `--screenshot=${join(outputRoot, `${scenario.name}.png`)}`, pageUrl],
    `screenshot capture for ${scenario.name}`,
  );
  return report;
}

function writeSummary(reports) {
  const summaryPath = join(outputRoot, 'summary.json');
  writeFileSync(summaryPath, JSON.stringify({ generatedAt: new Date().toISOString(), reports }, null, 2));
  return summaryPath;
}

function printReport(report) {
  const prefix = report.ok ? 'PASS' : 'FAIL';
  const warningCount = Object.values(report.warnings).reduce((count, value) => count + value.length, 0);
  const suffix = warningCount ? ` (${warningCount} warning${warningCount === 1 ? '' : 's'})` : '';
  console.log(`${prefix} ${report.scenario.name} ${report.viewport.width}x${report.viewport.height} layout=${report.layout}${suffix}`);
  if (!report.ok) {
    console.log(JSON.stringify(report.failures, null, 2));
  }
}

async function main() {
  requireFile(templatePath, 'remote-next template');
  rmSync(outputRoot, { recursive: true, force: true });
  mkdirSync(outputRoot, { recursive: true });

  const template = readFileSync(templatePath, 'utf8');
  const reports = [];
  for (const scenario of scenarios) {
    const scenarioDir = join(outputRoot, scenario.name);
    mkdirSync(scenarioDir, { recursive: true });
    const html = makeScenarioHtml(template, scenario);
    const htmlPath = join(scenarioDir, 'index.html');
    writeFileSync(htmlPath, html);
    const report = await validateScenario(scenario, pathToFileURL(htmlPath).href, scenarioDir);
    reports.push(report);
    printReport(report);
  }

  const summaryPath = writeSummary(reports);
  const failed = reports.filter((report) => !report.ok);
  console.log(`Output: ${outputRoot}`);
  console.log(`Summary: ${summaryPath}`);
  if (failed.length > 0) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
