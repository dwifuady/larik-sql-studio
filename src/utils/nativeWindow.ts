// Windows 11 native window integration (Mica backdrop + theme sync).
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/**
 * Ask the backend whether the Mica backdrop was applied and, if so, flag the
 * document so the CSS switches the window chrome to translucent surfaces.
 * Falls back silently to opaque surfaces (Windows 10, non-Windows, browser dev).
 */
export async function initNativeMaterial(): Promise<void> {
  try {
    const mica = await invoke<boolean>('is_mica_enabled');
    if (mica) document.documentElement.setAttribute('data-mica', '');
  } catch {
    // Not running inside Tauri, or command unavailable.
  }
}

/** Keep the native window theme (and therefore the Mica tint) in step with the app theme. */
export function syncNativeTheme(theme: 'dark' | 'light' | 'system'): void {
  try {
    void getCurrentWindow()
      .setTheme(theme === 'system' ? null : theme)
      .catch(() => undefined);
  } catch {
    // Not running inside Tauri.
  }
}
