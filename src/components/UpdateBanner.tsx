import { openUrl } from '@tauri-apps/plugin-opener';
import { useUpdateCheck } from '../hooks/useUpdateCheck';

/**
 * Notify-only update banner. Shows when a newer GitHub release exists.
 * "Download" opens the release page in the default browser (works for
 * portable + installed builds; no self-install).
 */
export function UpdateBanner() {
  const update = useUpdateCheck();

  if (!update) return null;

  return (
    <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 flex items-center gap-3 px-4 py-2.5 rounded-lg border border-blue-500/30 bg-blue-500/10 text-blue-400 backdrop-blur-md shadow-lg animate-slide-in-right">
      <svg className="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v12m0 0l-4-4m4 4l4-4M4 20h16" />
      </svg>
      <span className="text-sm font-medium">Version {update.version} available</span>
      <button
        onClick={() => openUrl(update.url)}
        className="text-sm font-semibold px-3 py-1 rounded-md bg-blue-500/20 hover:bg-blue-500/30 transition-colors"
      >
        Download
      </button>
    </div>
  );
}
