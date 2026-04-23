import { describe, expect, it } from 'vitest';
import { booleanArg, numericArg, parseTciText, trailingArg } from '../src/tci/parser';

describe('parseTciText', () => {
  it('parses semicolon separated commands', () => {
    expect(parseTciText('ready;vfo:0,0,14200000;trx:0,false;')).toEqual([
      { name: 'ready', args: [], raw: 'ready' },
      { name: 'vfo', args: ['0', '0', '14200000'], raw: 'vfo:0,0,14200000' },
      { name: 'trx', args: ['0', 'false'], raw: 'trx:0,false' },
    ]);
  });

  it('ignores empty commands and normalizes names', () => {
    expect(parseTciText(' ; RX_SMETER:0,0,-87.5;; ')).toEqual([
      { name: 'rx_smeter', args: ['0', '0', '-87.5'], raw: 'RX_SMETER:0,0,-87.5' },
    ]);
  });
});

describe('argument helpers', () => {
  it('parses boolean variants', () => {
    expect(booleanArg('true')).toBe(true);
    expect(booleanArg('0')).toBe(false);
    expect(booleanArg('maybe')).toBeNull();
  });

  it('parses numeric values and trailing args', () => {
    expect(numericArg(' 384000 ')).toBe(384000);
    expect(numericArg('abc')).toBeNull();
    expect(trailingArg(['0', '1', '2'])).toBe('2');
  });
});
