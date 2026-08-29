import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/p23test.html'),
  'utf8',
);

describe('Saturn Go performance lab template', () => {
  it('provides a fixed-window benchmark workflow and persistent A/B comparison', () => {
    for (const id of [
      'performance-lab-card',
      'tab-performance',
      'panel-performance',
      'benchmark-start',
      'benchmark-abort',
      'benchmark-duration',
      'benchmark-warmup',
      'benchmark-observation',
      'benchmark-baseline',
      'benchmark-candidate',
      'benchmark-compare',
      'benchmark-history',
    ]) {
      expect(template).toContain(`id="${id}"`);
    }

    expect(template).toContain("fetch('./performance_benchmarks'");
    expect(template).toContain("fetch('./performance_benchmarks/compare'");
    expect(template).toContain('benchmarkConsumeSample(perf, d)');
    expect(template).toContain('same workload identity');
    expect(template).toContain('Sounded clean');
    expect(template).toContain('running_exe_sha256');
    expect(template).toContain('artifact_sha256');
    expect(template).toContain("backend === 'xdma' ? 'Saturn Bridge' : 'P2app'");
    expect(template).toContain('perf?.service?.running_exe_sha256');
    expect(template).toContain('currentBackend !== active.backend');
    expect(template).toContain('Open Saturn Remote and keep RX streaming.');
    expect(template).toContain('Connect Thetis or another Protocol 2 client and start the radio.');
    expect(template).toContain("showLabPanel('performance')");
  });
});
