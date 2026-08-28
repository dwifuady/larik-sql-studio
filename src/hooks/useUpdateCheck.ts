import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';

// ponytail: hardcoded repo slug. Move to tauri.conf/env if repo ever renamed/forked.
const REPO = 'dwifuady/larik-sql-studio';
const RELEASES_URL = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASE_PAGE = `https://github.com/${REPO}/releases/latest`;

export interface UpdateInfo {
  version: string; // latest version, no leading "v"
  url: string; // release page to open
}

/** Parse "1.2.3" -> [1,2,3]; non-numeric segments -> 0. */
function parseSemver(v: string): [number, number, number] {
  const clean = v.replace(/^v/, '').split('-')[0].split('+')[0];
  const parts = clean.split('.').map((p) => parseInt(p, 10) || 0);
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
}

/** true if `latest` is strictly newer than `current`. */
function isNewer(latest: string, current: string): boolean {
  const a = parseSemver(latest);
  const b = parseSemver(current);
  for (let i = 0; i < 3; i++) {
    if (a[i] > b[i]) return true;
    if (a[i] < b[i]) return false;
  }
  return false;
}

/**
 * Checks GitHub Releases for a newer version once on mount.
 * Notify-only: does not download/install (works for portable + installed).
 * Silent on any failure (offline, rate-limited, no releases yet).
 */
export function useUpdateCheck(): UpdateInfo | null {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const current = await getVersion();
        const res = await fetch(RELEASES_URL, {
          headers: { Accept: 'application/vnd.github+json' },
        });
        if (!res.ok) return; // 404 (no releases), 403 (rate limit), etc.
        const data: { tag_name?: string; html_url?: string } = await res.json();
        const tag = data.tag_name;
        if (!tag || !isNewer(tag, current)) return;
        if (cancelled) return;
        setUpdate({ version: tag.replace(/^v/, ''), url: data.html_url || RELEASE_PAGE });
      } catch {
        // network/parse failure -> stay silent, no nag.
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return update;
}
