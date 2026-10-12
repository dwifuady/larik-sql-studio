// Root of a pop-out window: one tab's editor and results in a small
// always-on-top window (see docs/popout-tab.md).
import { useCallback, useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emitTo } from '@tauri-apps/api/event';
import { ChevronsDown, ChevronsUp, Pin, PinOff } from 'lucide-react';
import { useAppStore } from '../store';
import * as api from '../api';
import { QueryEditor } from './QueryEditor';
import { ToastContainer } from './Toast';
import { defaultDatabaseFor, spaceHasConnection } from '../types';
import { getReadableTextColor } from '../utils/color';
import { matchesShortcut } from '../hooks/useKeyboardShortcuts';
import { flushPendingAutosave } from '../utils/pendingAutosave';
import {
  EVENT_POPOUT_DOCK,
  EVENT_POPOUT_INIT,
  EVENT_POPOUT_READY,
  MAIN_WINDOW_LABEL,
  applyPopoutPayload,
  buildPopoutPayload,
  popoutTitle,
  type PopoutResultsPayload,
} from '../utils/popout';

const EVENT_EDITOR_ACTION = 'larik:editor-action';

function dispatchEditorAction(action: string) {
  window.dispatchEvent(new CustomEvent(EVENT_EDITOR_ACTION, { detail: { action } }));
}

/** First non-blank line of the query, for the collapsed editor bar. */
function firstLine(content: string | null): string {
  return (content ?? '').split(/\r?\n/).find(line => line.trim())?.trim() ?? '';
}

export function PopoutApp({ tabId }: { tabId: string }) {
  const tab = useAppStore(s => s.tabs.find(t => t.id === tabId) ?? null);
  const space = useAppStore(s => (tab ? s.spaces.find(sp => sp.id === tab.space_id) ?? null : null));
  const spaceDatabases = useAppStore(s => s.spaceDatabases);
  const isExecuting = useAppStore(s => s.tabExecuting[tabId] ?? false);
  const hasSelection = useAppStore(s => s.activeTabHasSelection);
  const cancelRunningQueries = useAppStore(s => s.cancelRunningQueries);

  const [status, setStatus] = useState<'loading' | 'ready' | 'missing'>('loading');
  const [editorCollapsed, setEditorCollapsed] = useState(false);
  const [pinned, setPinned] = useState(true);

  const spaceColor = space?.color || '#0078d4';
  const hasConnection = space ? spaceHasConnection(space) : false;
  const title = tab
    ? popoutTitle(space?.name, tab.title, tab.database || defaultDatabaseFor(space, spaceDatabases))
    : 'Pop-out';

  // Load the tab and its space, then ask the main window for the tab's results
  useEffect(() => {
    document.getElementById('html-loader')?.classList.add('hidden');

    const store = useAppStore.getState();
    store.initTheme();

    let unlistenInit: (() => void) | undefined;
    let cancelled = false;

    const init = async () => {
      unlistenInit = await getCurrentWebviewWindow().listen<PopoutResultsPayload>(EVENT_POPOUT_INIT, (event) => {
        if (event.payload.tabId !== tabId) return;
        useAppStore.setState(state => applyPopoutPayload(state, event.payload));
      });

      await store.loadAppSettings();
      const [loadedTab, spaces] = await Promise.all([api.getTab(tabId), api.getSpaces()]);
      if (cancelled) return;
      if (!loadedTab) {
        setStatus('missing');
        return;
      }

      // Pin the active space to the tab's space: the editor and the query
      // actions read the active space for the connection and colour.
      useAppStore.setState({
        spaces,
        activeSpaceId: loadedTab.space_id,
        tabs: [loadedTab],
        activeTabId: loadedTab.id,
      });
      setStatus('ready');

      await emitTo(MAIN_WINDOW_LABEL, EVENT_POPOUT_READY, { tabId });

      // The connection lives in the backend, so it is usually already open
      await store.refreshSpaceConnectionStatus();
      const loadedSpace = spaces.find(s => s.id === loadedTab.space_id);
      if (loadedSpace && spaceHasConnection(loadedSpace) && !useAppStore.getState().isConnected()) {
        await store.connectToSpace();
      }
    };

    init().catch((error) => {
      console.error('[Popout] Initialisation failed:', error);
      if (!cancelled) setStatus('missing');
    });

    return () => {
      cancelled = true;
      unlistenInit?.();
    };
  }, [tabId]);

  // Follow theme changes made in the main window
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'larik-theme' && e.newValue) {
        useAppStore.getState().setTheme(e.newValue as 'dark' | 'light' | 'system');
      }
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  // Closing the window docks the tab back into the main window
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWindow().onCloseRequested(async () => {
      try {
        const state = useAppStore.getState();
        if (state.tabExecuting[tabId]) {
          await state.cancelRunningQueries(tabId);
        }
        await flushPendingAutosave(tabId);
        const latest = useAppStore.getState();
        await emitTo(MAIN_WINDOW_LABEL, EVENT_POPOUT_DOCK, buildPopoutPayload(latest, tabId, latest.maxResultRows));
      } catch (error) {
        // Never block the close; the main window also undocks when this window is destroyed
        console.error('[Popout] Failed to dock back:', error);
      }
    }).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [tabId]);

  const closeWindow = useCallback(() => {
    void getCurrentWindow().close();
  }, []);

  const togglePinned = useCallback(async () => {
    const next = !pinned;
    try {
      await getCurrentWindow().setAlwaysOnTop(next);
      setPinned(next);
    } catch (error) {
      console.error('[Popout] Failed to toggle always on top:', error);
    }
  }, [pinned]);

  // Window-level shortcuts. Query execution inside the editor is handled by
  // Monaco's own actions; these cover the rest of the window.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const inEditor = (e.target as HTMLElement | null)?.closest('.monaco-editor') !== null;

      if (matchesShortcut(e, 'Ctrl+W') || matchesShortcut(e, 'Ctrl+Alt+O')) {
        e.preventDefault();
        closeWindow();
        return;
      }
      if (matchesShortcut(e, 'Ctrl+R')) {
        e.preventDefault();
        setEditorCollapsed(c => !c);
        return;
      }
      if (matchesShortcut(e, 'F5') || matchesShortcut(e, 'Ctrl+Enter')) {
        // Prevent a page reload; in the editor Monaco runs the query itself
        if (matchesShortcut(e, 'F5')) e.preventDefault();
        if (!inEditor) {
          e.preventDefault();
          dispatchEditorAction('run');
        }
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [closeWindow]);

  return (
    <div className="flex flex-col h-full w-full overflow-hidden bg-[var(--bg-primary)]">
      {/* Title bar */}
      <div
        data-tauri-drag-region
        className="h-8 shrink-0 flex items-center gap-2 pl-3 select-none border-b border-[var(--border-subtle)]"
        style={{ boxShadow: `inset 0 2px 0 ${spaceColor}` }}
      >
        <span className="w-2 h-2 rounded-full shrink-0 pointer-events-none" style={{ backgroundColor: spaceColor }} />
        <span data-tauri-drag-region className="flex-1 min-w-0 truncate text-xs font-medium text-[var(--text-primary)]" title={title}>
          {title}
        </span>

        {status === 'ready' && (
          <div className="flex items-center gap-0.5 shrink-0">
            <button
              type="button"
              onClick={() => setEditorCollapsed(c => !c)}
              className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
              title={editorCollapsed ? 'Show the editor (Ctrl+R)' : 'Hide the editor so the results fill the window (Ctrl+R)'}
              aria-label={editorCollapsed ? 'Show the editor' : 'Hide the editor'}
            >
              {editorCollapsed ? <ChevronsDown className="w-3.5 h-3.5" /> : <ChevronsUp className="w-3.5 h-3.5" />}
            </button>

            {isExecuting ? (
              <button
                type="button"
                onClick={() => void cancelRunningQueries(tabId)}
                className="p-1 text-white rounded-md transition-all hover:brightness-110 active:scale-95 bg-red-500"
                title="Cancel Query"
                aria-label="Cancel query"
              >
                <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            ) : (
              <button
                type="button"
                onClick={() => dispatchEditorAction('run')}
                disabled={!hasConnection}
                className="p-1 rounded-md transition-all hover:brightness-110 active:scale-95 disabled:opacity-40 disabled:cursor-not-allowed"
                style={{
                  backgroundColor: hasConnection ? spaceColor : 'transparent',
                  color: hasConnection ? getReadableTextColor(spaceColor) : 'var(--text-muted)',
                }}
                title={
                  !hasConnection
                    ? 'Configure a connection first'
                    : hasSelection ? 'Execute selected text (Ctrl+Enter)' : 'Execute query (Ctrl+Enter)'
                }
                aria-label="Execute query"
              >
                <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M8 5v14l11-7z" />
                </svg>
              </button>
            )}

            <button
              type="button"
              onClick={() => void togglePinned()}
              className={`p-1 rounded-md transition-colors ${pinned
                ? 'text-[var(--text-primary)] bg-[var(--bg-active)]'
                : 'text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]'
                }`}
              title={pinned ? 'Always on top. Click to unpin' : 'Pin on top of other windows'}
              aria-label={pinned ? 'Unpin window' : 'Pin window on top'}
              aria-pressed={pinned}
            >
              {pinned ? <Pin className="w-3.5 h-3.5" /> : <PinOff className="w-3.5 h-3.5" />}
            </button>
          </div>
        )}

        {/* Window controls */}
        <div className="flex items-stretch h-full shrink-0">
          <button
            type="button"
            onClick={() => void getCurrentWindow().minimize()}
            className="w-[40px] flex items-center justify-center text-[var(--text-primary)] hover:bg-[var(--bg-hover)] active:bg-[var(--bg-active)]"
            title="Minimize"
            aria-label="Minimize"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1">
              <path d="M0 5.5h10" />
            </svg>
          </button>
          <button
            type="button"
            onClick={closeWindow}
            className="w-[40px] flex items-center justify-center text-[var(--text-primary)] hover:bg-[#c42b1c] hover:text-white active:bg-[#c42b1c]/80"
            title="Close and dock back to the main window (Ctrl+W)"
            aria-label="Close and dock back"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1">
              <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
            </svg>
          </button>
        </div>
      </div>

      {/* Collapsed editor: one line of the query, click to expand */}
      {status === 'ready' && tab && editorCollapsed && (
        <button
          type="button"
          onClick={() => setEditorCollapsed(false)}
          className="shrink-0 mx-1.5 mt-1.5 px-2.5 py-1 rounded-md border border-[var(--border-subtle)] text-left font-mono text-[11px] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] truncate transition-colors"
          title="Show the editor (Ctrl+R)"
        >
          {firstLine(tab.content) || 'Empty query'}
        </button>
      )}

      <main className="flex-1 flex flex-col min-h-0 min-w-0">
        {status === 'ready' && tab ? (
          <QueryEditor tab={tab} popout editorCollapsed={editorCollapsed} />
        ) : status === 'missing' ? (
          <div className="flex-1 flex flex-col items-center justify-center gap-3 text-sm text-[var(--text-secondary)]">
            This tab no longer exists.
            <button
              type="button"
              onClick={closeWindow}
              className="px-3 py-1.5 rounded-md bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
            >
              Close
            </button>
          </div>
        ) : (
          <div className="flex-1 flex items-center justify-center text-sm text-[var(--text-muted)]">
            Loading…
          </div>
        )}
      </main>

      <ToastContainer />
    </div>
  );
}
