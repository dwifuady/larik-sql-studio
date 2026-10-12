// Shows the native query a driver generated for a result — e.g. the
// aggregation pipeline compiled from a SQL SELECT on MongoDB.
import { useState } from 'react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { useAppStore } from '../store';

interface GeneratedQueryBarProps {
  query: string;
  /** Database for the tab opened from the pipeline */
  database: string | null;
}

export function GeneratedQueryBar({ query, database }: GeneratedQueryBarProps) {
  const [expanded, setExpanded] = useState(false);
  const addToast = useAppStore(s => s.addToast);
  const createTab = useAppStore(s => s.createTab);

  const handleCopy = async () => {
    try {
      await writeText(query);
      addToast({ type: 'success', message: 'Pipeline copied' });
    } catch {
      addToast({ type: 'error', message: 'Could not copy the pipeline' });
    }
  };

  const handleOpen = async () => {
    await createTab('Pipeline', 'query', query, database);
  };

  const buttonClass = 'px-1.5 py-0.5 rounded hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors';

  return (
    <div className="flex-shrink-0 border-b border-[var(--border-color)] text-xs text-[var(--text-muted)]">
      <div className="flex items-center gap-1 px-3 py-1">
        <button
          onClick={() => setExpanded(e => !e)}
          className={`${buttonClass} flex items-center gap-1`}
          aria-expanded={expanded}
          title="The aggregation pipeline this SQL ran as"
        >
          <svg className={`w-3 h-3 transition-transform ${expanded ? 'rotate-90' : ''}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          Ran as aggregation pipeline
        </button>
        <span className="flex-1" />
        <button onClick={handleCopy} className={buttonClass}>Copy</button>
        <button onClick={handleOpen} className={buttonClass} title="Open the pipeline in a new query tab">Open in tab</button>
      </div>
      {expanded && (
        <pre className="mx-3 mb-2 p-2 max-h-48 overflow-auto rounded-md bg-[var(--bg-secondary)] text-[var(--text-primary)] font-mono text-[11px] leading-relaxed whitespace-pre select-text">
          {query}
        </pre>
      )}
    </div>
  );
}
