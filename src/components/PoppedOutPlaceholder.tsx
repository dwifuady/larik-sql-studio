// Shown in the main window in place of the editor for a tab that lives in a pop-out window.
import { ExternalLink, PanelLeftClose } from 'lucide-react';
import { useAppStore } from '../store';
import type { Tab } from '../types';

export function PoppedOutPlaceholder({ tab }: { tab: Tab }) {
  const focusPopout = useAppStore(s => s.focusPopout);
  const closePopout = useAppStore(s => s.closePopout);

  return (
    <div className="flex-1 flex items-center justify-center">
      <div className="text-center max-w-sm px-6">
        <div className="w-16 h-16 rounded-2xl bg-[var(--bg-hover)] flex items-center justify-center mx-auto mb-4">
          <ExternalLink className="w-7 h-7 text-[var(--text-muted)]" />
        </div>
        <p className="text-[var(--text-secondary)] mb-1">"{tab.title}" is open in a pop-out window</p>
        <p className="text-[var(--text-muted)] text-sm mb-5">Close the pop-out to bring it back here.</p>
        <div className="flex items-center justify-center gap-2">
          <button
            type="button"
            onClick={() => void focusPopout(tab.id)}
            className="px-3 py-1.5 text-sm rounded-md bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
          >
            Show pop-out
          </button>
          <button
            type="button"
            onClick={() => void closePopout(tab.id)}
            className="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
          >
            <PanelLeftClose className="w-4 h-4" />
            Dock back
          </button>
        </div>
      </div>
    </div>
  );
}
