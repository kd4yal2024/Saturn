import { createDefaultSettingsState } from '../settings/defaults';
import { applyRemoteSettingsToState, settingsStateToRemoteSettings } from '../settings/normalize';
import type { SettingsState, WsUrlEnvironment } from '../settings/types';

export type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

export const STORAGE_KEYS = {
  settings: 'saturn.remote.settings',
} as const;

export function loadSettingsFromStorage(storage: StorageLike, env: WsUrlEnvironment): SettingsState {
  const state = createDefaultSettingsState();
  const raw = storage.getItem(STORAGE_KEYS.settings);
  if (!raw) return state;

  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    applyRemoteSettingsToState(parsed, state, env);
    return state;
  } catch {
    return state;
  }
}

export function saveSettingsToStorage(storage: StorageLike, state: SettingsState, env: WsUrlEnvironment): void {
  storage.setItem(STORAGE_KEYS.settings, JSON.stringify(settingsStateToRemoteSettings(state, env)));
}
