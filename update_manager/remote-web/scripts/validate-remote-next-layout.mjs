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

const viewportScenarios = [
  { name: 'phone-portrait', width: 390, height: 844, layout: 'phone' },
  { name: 'phone-landscape', width: 844, height: 390, layout: 'phone' },
  { name: 'tablet-portrait', width: 768, height: 1024, layout: 'desktop' },
  { name: 'tablet-landscape', width: 1024, height: 768, layout: 'desktop' },
  { name: 'compact-desktop', width: 1366, height: 768, layout: 'desktop' },
  { name: 'desktop', width: 1440, height: 900, layout: 'desktop' },
  { name: 'desktop-hd', width: 1920, height: 1080, layout: 'desktop' },
  { name: 'desktop-ultrawide', width: 2560, height: 1440, layout: 'desktop' },
];

const drawerScenarios = viewportScenarios.flatMap((scenario) => [
  { ...scenario, drawerOpen: false },
  { ...scenario, name: `${scenario.name}-drawer`, drawerOpen: true },
]);
const setupScenarios = viewportScenarios
  .filter((scenario) => ['phone-portrait', 'phone-landscape', 'tablet-portrait', 'desktop', 'desktop-hd'].includes(scenario.name))
  .map((scenario) => ({ ...scenario, name: `${scenario.name}-setup`, drawerOpen: false, setupOpen: true }));
const operationsScenarios = viewportScenarios
  .filter((scenario) => scenario.name === 'desktop-hd')
  .map((scenario) => ({
    ...scenario,
    name: `${scenario.name}-operations-audio`,
    drawerOpen: false,
    setupOpen: false,
    operationsAudioOpen: true,
  }));
const allScenarios = [...drawerScenarios, ...setupScenarios, ...operationsScenarios];
const requestedScenarioNames = new Set(
  `${process.env.SATURN_LAYOUT_SCENARIOS || ''}`
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean),
);
const scenarios = requestedScenarioNames.size
  ? allScenarios.filter((scenario) => requestedScenarioNames.has(scenario.name))
  : allScenarios;

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
  function drawerFailures() {
    const overlay = document.getElementById("operator-detail-overlay");
    const sheet = document.querySelector(".operator-detail-sheet");
    const grid = document.getElementById("operator-detail-grid");
    const overlayVisible = overlay ? visible(overlay) : false;
    if (!scenario.drawerOpen) {
      return {
        drawerMissing: overlay ? [] : ["operator-detail-overlay"],
        drawerUnexpectedlyVisible: overlayVisible ? ["operator-detail-overlay"] : [],
        drawerViewportOverflow: [],
        drawerContentOverflow: [],
        drawerGroupOverlaps: [],
        drawerGroupCount: [],
        drawerRowCount: []
      };
    }

    const sheetRect = sheet ? rectFor(sheet) : null;
    const groups = Array.from(document.querySelectorAll(".operator-detail-group"));
    const rows = Array.from(document.querySelectorAll(".operator-detail-row"));
    const groupBoxes = groups.map((element) => ({ id: element.querySelector(".operator-detail-group-title")?.textContent.trim() || "", rect: rectFor(element), visible: visible(element) }));
    const groupOverlaps = [];
    for (let i = 0; i < groupBoxes.length; i += 1) {
      for (let j = i + 1; j < groupBoxes.length; j += 1) {
        if (overlap(groupBoxes[i].rect, groupBoxes[j].rect)) {
          groupOverlaps.push([groupBoxes[i].id, groupBoxes[j].id]);
        }
      }
    }

    return {
      drawerMissing: [
        overlay ? "" : "operator-detail-overlay",
        sheet ? "" : "operator-detail-sheet",
        grid ? "" : "operator-detail-grid"
      ].filter(Boolean),
      drawerHidden: overlayVisible ? [] : ["operator-detail-overlay"],
      drawerViewportOverflow: sheetRect && (
        sheetRect.left < -1 ||
        sheetRect.top < -1 ||
        sheetRect.right > window.innerWidth + 1 ||
        sheetRect.bottom > window.innerHeight + 1
      ) ? ["operator-detail-sheet"] : [],
      drawerContentOverflow: textOverflow(".operator-detail-title, .operator-detail-subtitle, .operator-detail-group-title, .operator-detail-label, .operator-detail-value, .operator-detail-note"),
      drawerGroupOverlaps: groupOverlaps,
      drawerGroupCount: groups.length >= 5 ? [] : [{ expectedAtLeast: 5, actual: groups.length }],
      drawerRowCount: rows.length >= 22 ? [] : [{ expectedAtLeast: 22, actual: rows.length }]
    };
  }
  function setupFailures() {
    const menu = document.getElementById("setup-menu");
    const nav = menu?.querySelector(".setup-nav");
    const tabs = Array.from(menu?.querySelectorAll("[data-setup-panel]") || []);
    const panels = Array.from(menu?.querySelectorAll("[data-setup-panel-id]") || []);
    if (!scenario.setupOpen) {
      return {
        setupMissing: menu ? [] : ["setup-menu"],
        setupUnexpectedlyVisible: menu && visible(menu) ? ["setup-menu"] : [],
        setupViewportOverflow: [],
        setupContentOverflow: [],
        setupTabCount: [],
        setupPanelCount: []
      };
    }
    const rect = menu ? rectFor(menu) : null;
    return {
      setupMissing: [menu ? "" : "setup-menu", nav ? "" : "setup-nav"].filter(Boolean),
      setupHidden: menu && visible(menu) ? [] : ["setup-menu"],
      setupViewportOverflow: rect && (
        rect.left < -1 || rect.top < -1 ||
        rect.right > window.innerWidth + 1 || rect.bottom > window.innerHeight + 1
      ) ? ["setup-menu"] : [],
      setupContentOverflow: textOverflow(".setup-title, .setup-meta, .setup-tab-btn, .setup-section-title, .setup-readout"),
      setupTabCount: tabs.length === 7 ? [] : [{ expected: 7, actual: tabs.length }],
      setupPanelCount: panels.length === 7 ? [] : [{ expected: 7, actual: panels.length }]
    };
  }
  function operationsAudioFailures() {
    if (!scenario.operationsAudioOpen) return {};
    const drawer = document.getElementById("operations-drawer");
    const panel = document.getElementById("operations-panel-audio");
    const close = document.getElementById("operations-audio-close-btn");
    const closeRect = close ? rectFor(close) : null;
    return {
      operationsAudioMissing: [drawer ? "" : "operations-drawer", panel ? "" : "operations-panel-audio", close ? "" : "operations-audio-close-btn"].filter(Boolean),
      operationsAudioHidden: [
        drawer && visible(drawer) ? "" : "operations-drawer",
        panel && visible(panel) ? "" : "operations-panel-audio",
        close && visible(close) ? "" : "operations-audio-close-btn"
      ].filter(Boolean),
      operationsAudioCloseViewportOverflow: closeRect && (
        closeRect.left < -1 || closeRect.top < -1 ||
        closeRect.right > window.innerWidth + 1 || closeRect.bottom > window.innerHeight + 1
      ) ? [{ id: "operations-audio-close-btn", rect: closeRect }] : []
    };
  }
  function displayWorkspaceFailures() {
    const stack = document.querySelector(".display-stack");
    const spectrum = document.getElementById("spectrum-shell");
    const waterfall = document.getElementById("waterfall-shell");
    const separator = document.getElementById("spectrum-waterfall-resizer");
    const stackRect = stack ? rectFor(stack) : null;
    const spectrumRect = spectrum ? rectFor(spectrum) : null;
    const waterfallRect = waterfall ? rectFor(waterfall) : null;
    const separatorVisible = separator ? visible(separator) : false;
    const aligned = spectrumRect && waterfallRect &&
      Math.abs(spectrumRect.left - waterfallRect.left) <= 1 &&
      Math.abs(spectrumRect.right - waterfallRect.right) <= 1;
    return {
      displayWorkspaceMissing: [
        stack ? "" : "display-stack",
        spectrum ? "" : "spectrum-shell",
        waterfall ? "" : "waterfall-shell",
        separator ? "" : "spectrum-waterfall-resizer"
      ].filter(Boolean),
      displayWorkspaceHidden: [
        spectrum && visible(spectrum) ? "" : "spectrum-shell",
        waterfall && visible(waterfall) ? "" : "waterfall-shell"
      ].filter(Boolean),
      displayWorkspaceMisaligned: aligned ? [] : [{ spectrum: spectrumRect, waterfall: waterfallRect }],
      displayWorkspaceTooSmall: [
        spectrumRect && spectrumRect.height >= 120 ? "" : { id: "spectrum-shell", height: spectrumRect?.height || 0 },
        waterfallRect && waterfallRect.height >= 80 ? "" : { id: "waterfall-shell", height: waterfallRect?.height || 0 }
      ].filter(Boolean),
      displayWorkspaceViewportOverflow: stackRect && (
        stackRect.left < -1 || stackRect.right > window.innerWidth + 1
      ) ? ["display-stack"] : [],
      displaySeparatorState: scenario.layout === "phone"
        ? (separatorVisible ? ["separator visible in phone mode"] : [])
        : (separatorVisible ? [] : ["separator hidden outside phone mode"])
    };
  }
  function runValidation() {
    const page = document.querySelector(".page.console-page");
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
    const pageRect = page ? rectFor(page) : null;
    const failures = {
      missing,
      invisible,
      geometryOverlaps,
      stripOverflow,
      viewportOverflow,
      valueOverflow: textOverflow(".operator-pill-value"),
      layoutMismatch: layout === scenario.layout ? [] : [{ expected: scenario.layout, actual: layout }],
      lcdViewportOverflow: scenario.name === "desktop-hd" && pageRect && pageRect.bottom > window.innerHeight + 1
        ? [{ pageBottom: pageRect.bottom, viewportHeight: window.innerHeight }]
        : [],
      ...displayWorkspaceFailures(),
      ...drawerFailures(),
      ...setupFailures(),
      ...operationsAudioFailures()
    };
    const ok = Object.values(failures).every((value) => Array.isArray(value) && value.length === 0);
    const report = {
      scenario,
      ok,
      layout,
      viewport: { width: window.innerWidth, height: window.innerHeight },
      page: page ? {
        rect: pageRect,
        width: window.getComputedStyle(page).width,
        maxWidth: window.getComputedStyle(page).maxWidth
      } : null,
      stripRect,
      drawer: {
        open: Boolean(scenario.drawerOpen),
        overlayRect: document.getElementById("operator-detail-overlay") ? rectFor(document.getElementById("operator-detail-overlay")) : null,
        sheetRect: document.querySelector(".operator-detail-sheet") ? rectFor(document.querySelector(".operator-detail-sheet")) : null,
        groupCount: document.querySelectorAll(".operator-detail-group").length,
        rowCount: document.querySelectorAll(".operator-detail-row").length
      },
      setup: {
        open: Boolean(scenario.setupOpen),
        menuRect: document.getElementById("setup-menu") ? rectFor(document.getElementById("setup-menu")) : null,
        tabCount: document.querySelectorAll("#setup-menu [data-setup-panel]").length,
        panelCount: document.querySelectorAll("#setup-menu [data-setup-panel-id]").length
      },
      operationsAudio: {
        open: Boolean(scenario.operationsAudioOpen),
        drawerRect: document.getElementById("operations-drawer") ? rectFor(document.getElementById("operations-drawer")) : null,
        panelRect: document.getElementById("operations-panel-audio") ? rectFor(document.getElementById("operations-panel-audio")) : null,
        closeRect: document.getElementById("operations-audio-close-btn") ? rectFor(document.getElementById("operations-audio-close-btn")) : null
      },
      displayWorkspace: {
        stackRect: document.querySelector(".display-stack") ? rectFor(document.querySelector(".display-stack")) : null,
        spectrumRect: document.getElementById("spectrum-shell") ? rectFor(document.getElementById("spectrum-shell")) : null,
        waterfallRect: document.getElementById("waterfall-shell") ? rectFor(document.getElementById("waterfall-shell")) : null
      },
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
  const drawerOpen = ${JSON.stringify(Boolean(scenario.drawerOpen))};
  const setupOpen = ${JSON.stringify(Boolean(scenario.setupOpen))};
  const operationsAudioOpen = ${JSON.stringify(Boolean(scenario.operationsAudioOpen))};
  const values = {
    "operator-conn": ["warn", "Reconnecting 12", "Static validation state"],
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
  const consoleLayout = document.querySelector(".console-layout");
  const rightRail = document.querySelector(".right-rail");
  const audioStrip = document.querySelector('[data-phone-panel="audio"]');
  if (consoleLayout && rightRail && audioStrip && !document.getElementById("radio-context-rail")) {
    const contextRail = document.createElement("aside");
    contextRail.id = "radio-context-rail";
    contextRail.className = "context-rail";
    contextRail.dataset.activeContext = "rx";
    contextRail.setAttribute("aria-label", "Radio controls");
    const tabs = document.createElement("div");
    tabs.className = "context-tabs";
    tabs.setAttribute("role", "tablist");
    tabs.setAttribute("aria-label", "Radio control context");
    for (const context of ["rx", "dsp", "tx"]) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "context-tab";
      button.dataset.controlContext = context;
      button.setAttribute("role", "tab");
      button.setAttribute("aria-selected", context === "rx" ? "true" : "false");
      button.textContent = context.toUpperCase();
      tabs.appendChild(button);
    }
    contextRail.appendChild(tabs);
    contextRail.appendChild(audioStrip);
    contextRail.appendChild(rightRail);
    consoleLayout.insertAdjacentElement("afterend", contextRail);
    audioStrip.querySelectorAll("[data-control-context]").forEach((control) => {
      control.hidden = control.dataset.controlContext !== "rx";
    });
    rightRail.dataset.contextCompact = "true";
  }
  function detailRow(label, value, note, tone = "rx") {
    const row = document.createElement("div");
    row.className = "operator-detail-row";
    row.dataset.tone = tone;
    const labelNode = document.createElement("div");
    labelNode.className = "operator-detail-label";
    labelNode.textContent = label;
    const valueWrap = document.createElement("div");
    const valueNode = document.createElement("div");
    valueNode.className = "operator-detail-value";
    valueNode.textContent = value;
    valueWrap.appendChild(valueNode);
    if (note) {
      const noteNode = document.createElement("div");
      noteNode.className = "operator-detail-note";
      noteNode.textContent = note;
      valueWrap.appendChild(noteNode);
    }
    row.appendChild(labelNode);
    row.appendChild(valueWrap);
    return row;
  }
  function detailGroup(title, rows) {
    const section = document.createElement("section");
    section.className = "operator-detail-group";
    const titleNode = document.createElement("div");
    titleNode.className = "operator-detail-group-title";
    titleNode.textContent = title;
    section.appendChild(titleNode);
    rows.forEach((row) => section.appendChild(detailRow(...row)));
    return section;
  }
  if (drawerOpen) {
    const overlay = document.getElementById("operator-detail-overlay");
    const subtitle = document.getElementById("operator-detail-subtitle");
    const grid = document.getElementById("operator-detail-grid");
    if (overlay) overlay.hidden = false;
    if (subtitle) subtitle.textContent = "Connected / Role pending / PTT ON AIR";
    if (grid) {
      grid.textContent = "";
      [
        detailGroup("State Strip", [
          ["Connection", "Connected", "Static validation state", "ok"],
          ["Ownership", "Role pending", "Static validation state", "warn"],
          ["RX/TX", "PTT ON AIR", "Static validation state", "keyed"],
          ["RF State", "RF disabled", "Static validation state", "alarm"],
          ["Transport", "Tailnet", "Static validation state", "rx"],
          ["Latency", "888 ms RTT", "Static validation state", "warn"],
          ["Audio", "48 ms lead", "Static validation state", "warn"],
          ["Fault", "Power trip", "Static validation state", "alarm"]
        ]),
        detailGroup("Radio", [
          ["VFO A", "14.200.000", "20m band", "ok"],
          ["Mode", "USB", "Sample rate 96 kHz", "rx"],
          ["RX Filter", "50-3050 Hz", "No shift", "rx"],
          ["Freq Lock", "Off", "QSY controls available", "ok"]
        ]),
        detailGroup("Link / Audio", [
          ["Bridge URL", "wss://saturn-g2.local/tci", "saturn-g2.local", "ok"],
          ["RTT", "888 ms", "Bridge websocket round trip", "warn"],
          ["Audio Lead", "48 ms", "0 resync/drop event(s)", "ok"],
          ["Audio Path", "MSG", "48 kHz RX audio", "rx"]
        ]),
        detailGroup("TX Safety", [
          ["RF Gate", "Disabled", "Bridge blocks RF TX", "warn"],
          ["Role", "viewer", "Client #2", "warn"],
          ["TX Ready", "Closed", "Bridge assigned viewer role", "warn"],
          ["TX Source", "MOX", "keyed", "keyed"],
          ["Latest Fault", "Power trip", "12s ago", "alarm"]
        ]),
        detailGroup("Fault History", [
          ["Latest", "Power trip", "Just now", "alarm"],
          ["Fault 2", "Bridge socket error", "8s ago", "warn"],
          ["Fault 3", "Mic unavailable", "22s ago", "alarm"],
          ["Fault 4", "Page hidden during TX", "46s ago", "warn"],
          ["Fault 5", "RF disabled", "1m ago", "warn"],
          ["Fault 6", "Role pending", "2m ago", "warn"],
          ["Fault 7", "TX idle timeout", "3m ago", "warn"],
          ["Fault 8", "Audio queue resync", "4m ago", "warn"]
        ])
      ].forEach((group) => grid.appendChild(group));
    }
  }
  if (operationsAudioOpen) {
    const drawer = document.getElementById("operations-drawer");
    if (drawer) {
      drawer.dataset.open = "true";
      drawer.dataset.activePanel = "audio";
    }
    document.querySelectorAll(".operations-tab").forEach((button) => {
      const active = button.dataset.operationsTarget === "audio";
      button.setAttribute("aria-selected", active ? "true" : "false");
      button.setAttribute("aria-expanded", active ? "true" : "false");
    });
    document.querySelectorAll("[data-operations-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.operationsPanel !== "audio";
    });
  }
  if (setupOpen) {
    const menu = document.getElementById("setup-menu");
    const scrim = document.getElementById("setup-scrim");
    if (menu) menu.hidden = false;
    if (scrim) scrim.hidden = false;
    document.body.classList.add("setup-open");
    document.querySelectorAll("#setup-menu [data-setup-panel]").forEach((button) => {
      const active = button.dataset.setupPanel === "display";
      button.classList.toggle("active", active);
      button.setAttribute("aria-selected", active ? "true" : "false");
    });
    document.querySelectorAll("#setup-menu [data-setup-panel-id]").forEach((panel) => {
      panel.hidden = panel.dataset.setupPanelId !== "display";
    });
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
    '--disable-breakpad',
    '--disable-crash-reporter',
    '--disable-default-apps',
    '--disable-features=Crashpad',
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
  if (!scenarios.length) {
    throw new Error(`No layout scenarios matched SATURN_LAYOUT_SCENARIOS=${[...requestedScenarioNames].join(',')}`);
  }
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
