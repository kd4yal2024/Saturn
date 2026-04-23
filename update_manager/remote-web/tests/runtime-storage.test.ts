import { describe, expect, it } from 'vitest';
import { createDefaultSettingsState } from '../src/settings/defaults';
import { loadSettingsFromStorage, saveSettingsToStorage, STORAGE_KEYS, type StorageLike } from '../src/runtime/storage';

const env = {
  protocol: 'https:',
  host: 'radio.local:8443',
  hostname: 'radio.local',
  href: 'https://radio.local:8443/remote',
};

function createMemoryStorage(initial: Record<string, string> = {}): StorageLike & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem(key: string) {
      return Object.prototype.hasOwnProperty.call(data, key) ? (data[key] ?? null) : null;
    },
    setItem(key: string, value: string) {
      data[key] = value;
    },
  };
}

describe('runtime storage', () => {
  it('loads defaults when storage is empty or invalid', () => {
    const empty = createMemoryStorage();
    expect(loadSettingsFromStorage(empty, env).displayZoom).toBe(1);

    const invalid = createMemoryStorage({ [STORAGE_KEYS.settings]: '{bad json' });
    expect(loadSettingsFromStorage(invalid, env).displayZoom).toBe(1);
  });

  it('round-trips normalized settings through storage', () => {
    const storage = createMemoryStorage();
    const state = createDefaultSettingsState();
    state.displayZoom = 4;
    state.activeProfile = 'Portable';
    state.radioPrefs.mode = 'DIGL';

    saveSettingsToStorage(storage, state, env);
    const loaded = loadSettingsFromStorage(storage, env);

    expect(loaded.displayZoom).toBe(4);
    expect(loaded.activeProfile).toBe('Portable');
    expect(loaded.radioPrefs.mode).toBe('DIGL');
  });
});
