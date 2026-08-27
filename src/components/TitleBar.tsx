// Custom window title bar for frameless window (T047)
import { getCurrentWindow } from '@tauri-apps/api/window';
import { GlobalSearch } from './GlobalSearch';
import { useState, useEffect, useRef } from 'react';
import { PanelLeft } from 'lucide-react';
import { DatabaseSelector } from './DatabaseSelector';
import { useAppStore } from '../store';
import { spaceHasConnection } from '../types';
import { getReadableTextColor } from '../utils/color';

interface TitleBarProps {
  sidebarWidth?: number;
  sidebarHidden?: boolean;
  onToggleSidebar?: () => void;
}

export function TitleBar({ sidebarWidth = 280, sidebarHidden = false, onToggleSidebar }: TitleBarProps) {
  const [isMaximized, setIsMaximized] = useState(false);
  const [titleVisible, setTitleVisible] = useState(true);
  const hideTimer = useRef<number | null>(null);
  const titleBarRef = useRef<HTMLDivElement>(null);
  const HIDE_DELAY = 1200;
  const appWindow = getCurrentWindow();
  const appInfo = useAppStore(s => s.appInfo);

  // Per-tab state for the merged editor toolbar (formerly in QueryEditor).
  const activeTabId = useAppStore(s => s.activeTabId);
  const tabs = useAppStore(s => s.tabs);
  const spaces = useAppStore(s => s.spaces);
  const activeSpaceId = useAppStore(s => s.activeSpaceId);
  const spaceConnectionStatus = useAppStore(s => s.spaceConnectionStatus);
  const isSaving = useAppStore(s => s.isSaving);

  const validationEnabled = useAppStore(s => s.validationEnabled);
  const toggleValidation = useAppStore(s => s.toggleValidation);
  const enableStickyNotes = useAppStore(s => s.enableStickyNotes);
  const activeTabHasSelection = useAppStore(s => s.activeTabHasSelection);
  const isExecuting = useAppStore(s => activeTabId ? (s.tabExecuting[activeTabId] ?? false) : false);
  const cancelRunningQueries = useAppStore(s => s.cancelRunningQueries);

  const revealTitleBar = () => {
    if (hideTimer.current) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
    setTitleVisible(true);
  };

  const scheduleHideTitleBar = () => {
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setTitleVisible(false), HIDE_DELAY);
  };

  // Reveal when the cursor enters the top edge of the window — throttled via rAF
  const rafRef = useRef<number | null>(null);
  useEffect(() => {
    const REVEAL_ZONE = 4;
    const onMouseMove = (e: MouseEvent) => {
      if (rafRef.current !== null) return;
      rafRef.current = window.requestAnimationFrame(() => {
        rafRef.current = null;
        if (e.clientY <= REVEAL_ZONE) {
          revealTitleBar();
        } else if (!titleBarRef.current?.contains(e.target as Node)) {
          if (!hideTimer.current) {
            scheduleHideTitleBar();
          }
        }
      });
    };
    window.addEventListener('mousemove', onMouseMove);
    return () => {
      window.removeEventListener('mousemove', onMouseMove);
      if (rafRef.current !== null) window.cancelAnimationFrame(rafRef.current);
    };
  }, []);

  // Auto-hide shortly after mount so users see the controls exist
  useEffect(() => {
    const t = window.setTimeout(() => setTitleVisible(false), 2500);
    return () => window.clearTimeout(t);
  }, []);

  // Cleanup pending hide timer on unmount
  useEffect(() => () => {
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
  }, []);

  // Check if window is maximized on mount
  useEffect(() => {
    const checkMaximized = async () => {
      const maximized = await appWindow.isMaximized();
      setIsMaximized(maximized);
    };
    checkMaximized();

    // Listen for window resize events
    const unlisten = appWindow.onResized(() => {
      checkMaximized();
    });

    return () => {
      unlisten.then(fn => fn());
    };
  }, [appWindow]);

  const handleMinimize = async () => {
    await appWindow.minimize();
  };

  const handleMaximize = async () => {
    await appWindow.toggleMaximize();
    const maximized = await appWindow.isMaximized();
    setIsMaximized(maximized);
  };

  const handleClose = async () => {
    await appWindow.close();
  };

  const currentSidebarWidth = sidebarHidden ? 0 : sidebarWidth;
  const titleText = appInfo.name ? `${appInfo.name}${appInfo.version ? ` v${appInfo.version}` : ''}` : 'Larik SQL Studio';

  // Derived per-tab info
  const activeTab = activeTabId ? tabs.find(t => t.id === activeTabId) : null;
  const activeSpace = activeSpaceId ? spaces.find(s => s.id === activeSpaceId) : null;
  const hasConnection = activeSpace ? spaceHasConnection(activeSpace) : false;
  const isConnected = spaceConnectionStatus?.is_connected ?? false;
  const spaceColor = activeSpace?.color || '#6366f1';

  const dispatchEditorAction = (action: 'run' | 'run-append' | 'format' | 'add-note') => {
    window.dispatchEvent(new CustomEvent('larik:editor-action', { detail: { action } }));
  };

  return (
    <div
      ref={titleBarRef}
      className={`flex items-center h-7 select-none relative pointer-events-auto transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] ${
        titleVisible
          ? 'translate-y-0 opacity-100'
          : '-translate-y-full opacity-0 pointer-events-none'
      }`}
      style={{ background: 'var(--bg-secondary)' }}
      onMouseEnter={revealTitleBar}
      onMouseLeave={scheduleHideTitleBar}
    >
      {/* Left section - Above sidebar or floating toggle when hidden */}
      <div
        className="relative h-full flex items-center transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)]"
        style={{
          width: sidebarHidden ? 'auto' : `${currentSidebarWidth}px`,
          paddingLeft: sidebarHidden ? '6px' : '0',
          gap: '6px'
        }}
      >
        {/* Sidebar Toggle Button */}
        <div className={`flex items-center h-full relative z-10 ${!sidebarHidden ? 'px-1.5' : ''}`}>
          <button
            onClick={onToggleSidebar}
            className="p-1 rounded-md hover:bg-black/10 dark:hover:bg-white/10 text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-all group flex items-center gap-1.5"
            title="Toggle Sidebar (Ctrl+Shift+S)"
          >
            <PanelLeft className="w-3 h-3" />
          </button>
        </div>

        {/* Database Selector when hidden */}
        {sidebarHidden && (
          <div className="flex items-center h-full animate-in fade-in slide-in-from-left-2 duration-300">
            <DatabaseSelector isCompact={true} />
          </div>
        )}
      </div>

      {/* Resize handle separator removed as per request */}

      {/* Right section - Above main content area (draggable) */}
      <div
        data-tauri-drag-region
        className="flex-1 h-full min-w-0 flex items-center gap-1.5 px-2 pr-[140px]"
      >
        {activeTab ? (
          <>
            {/* Tab icon with space color */}
            <div
              className="w-4 h-4 rounded-md flex items-center justify-center flex-shrink-0"
              style={{ backgroundColor: `${spaceColor}20` }}
            >
              <svg
                className="w-2.5 h-2.5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
                style={{ color: spaceColor }}
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
              </svg>
            </div>

            <span className="text-[11px] font-medium text-[var(--text-primary)] truncate max-w-[140px]">
              {activeTab.title}
            </span>

            {activeTab.is_pinned && <span className="text-[10px]" title="Pinned">📌</span>}

            <span className="text-[10px] text-[var(--text-muted)]">•</span>

            <span className="text-[10px] truncate flex items-center gap-1">
              {isConnected ? (
                <>
                  <span className="text-[var(--text-secondary)] truncate max-w-[160px]">{activeSpace?.connection_username}@{activeSpace?.connection_host}</span>
                  <span className="text-[9px] font-medium px-1.5 py-0.5 rounded-full bg-green-500/15 text-green-400">Connected</span>
                </>
              ) : hasConnection ? (
                <>
                  <span className="text-[var(--text-secondary)] truncate max-w-[160px]">{activeSpace?.connection_username}@{activeSpace?.connection_host}</span>
                  <span className="text-[9px] font-medium px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-400">Disconnected</span>
                </>
              ) : (
                <span className="text-[9px] font-medium px-1.5 py-0.5 rounded-full bg-[var(--bg-hover)] text-[var(--text-muted)]">No connection</span>
              )}
            </span>

            {isSaving && (
              <div className="flex items-center gap-1 px-1.5 py-0.5 rounded-full bg-[var(--bg-hover)] text-[var(--text-muted)] text-[10px]">
                <div className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
                Saving
              </div>
            )}
          </>
        ) : (
          <div className="min-w-0 truncate text-[11px] font-medium text-[var(--text-muted)]">
            {titleText}
          </div>
        )}

        {/* Spacer pushes action buttons + search to the right */}
        <div className="ml-auto" />

        {/* Action buttons (formerly in editor toolbar) - in-flow, left of search */}
        {activeTab && (
          <div className="flex items-center gap-0.5 pointer-events-auto shrink-0">
            {/* Format button */}
            <button
              onClick={() => dispatchEditorAction('format')}
              className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
              title="Format SQL (Ctrl+Alt+F)"
            >
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h7" />
              </svg>
            </button>

            {/* Validation toggle button */}
            <button
              onClick={toggleValidation}
              className={`p-1 rounded-md transition-colors ${validationEnabled
                ? 'text-green-400 hover:text-green-300 bg-green-400/10 hover:bg-green-400/20'
                : 'text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]'
                }`}
              title={validationEnabled ? 'Validation enabled - click to disable' : 'Validation disabled - click to enable'}
            >
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                {validationEnabled ? (
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                ) : (
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
                )}
              </svg>
            </button>

            {/* Sticky Note toggle/add button */}
            {enableStickyNotes && (
              <button
                onClick={() => dispatchEditorAction('add-note')}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
                title="Add sticky note at cursor line"
              >
                <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M15.5 3H5a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2V8.5L15.5 3z" />
                  <path strokeLinecap="round" strokeLinejoin="round" d="M15 3v6h6" />
                </svg>
              </button>
            )}

            {/* Run/Cancel button */}
            {isExecuting ? (
              <button
                onClick={() => activeTabId && cancelRunningQueries(activeTabId)}
                className="p-1 text-white rounded-md transition-all hover:brightness-110 active:scale-95 bg-red-500"
                title="Cancel Query"
              >
                <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            ) : (
              <>
                {/* Run in new result tab (Ctrl+\) */}
                <button
                  onClick={() => dispatchEditorAction('run-append')}
                  disabled={!hasConnection}
                  className="p-[5px] rounded-md transition-all hover:brightness-110 active:scale-95 disabled:opacity-40 disabled:cursor-not-allowed relative"
                  style={{
                    backgroundColor: hasConnection ? `${spaceColor}18` : 'transparent',
                    color: hasConnection ? spaceColor : 'var(--text-muted)',
                  }}
                  title={
                    !hasConnection
                      ? 'Configure a connection first'
                      : 'Execute query in new result tab (Ctrl+\\)'
                  }
                >
                  <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M8 5v14l11-7z" />
                  </svg>
                  <svg className="w-2 h-2 absolute -top-[1px] -right-[1px] text-white"
                    fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={4}>
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>

                {/* Run in current tab (Ctrl+Enter) */}
                <button
                  onClick={() => dispatchEditorAction('run')}
                  disabled={!hasConnection}
                  className="p-1 text-white rounded-md transition-all hover:brightness-110 active:scale-95 disabled:opacity-40 disabled:cursor-not-allowed"
                  style={{
                    backgroundColor: hasConnection ? spaceColor : 'transparent',
                    color: hasConnection ? getReadableTextColor(spaceColor) : 'var(--text-muted)',
                  }}
                  title={
                    !hasConnection
                      ? 'Configure a connection first'
                      : activeTabHasSelection
                        ? 'Execute selected text (Ctrl+Enter)'
                        : 'Execute query (Ctrl+Enter)'
                  }
                >
                  <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M8 5v14l11-7z" />
                  </svg>
                </button>
              </>
            )}
          </div>
        )}

        {/* Search Bar - aligned with content flow */}
        <div className="mr-2 pointer-events-auto shrink-0 min-w-[12rem] max-w-md">
          <GlobalSearch />
        </div>
      </div>

      {/* Window controls - Absolute positioned on the right */}
      <div className="absolute right-0 top-0 h-full flex items-center">
        {/* Minimize button */}
        <button
          type="button"
          onClick={handleMinimize}
          className="h-full px-3 flex items-center justify-center hover:bg-white/5 transition-colors group"
          title="Minimize"
        >
          <svg
            className="w-3 h-3 text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M19 12H5"
            />
          </svg>
        </button>

        {/* Maximize/Restore button */}
        <button
          type="button"
          onClick={handleMaximize}
          className="h-full px-3 flex items-center justify-center hover:bg-white/5 transition-colors group"
          title={isMaximized ? 'Restore' : 'Maximize'}
        >
          {isMaximized ? (
            <svg
              className="w-3 h-3 text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 9V4.5M9 9H4.5M9 9L3.75 3.75M9 15v4.5M9 15H4.5M9 15l-5.25 5.25M15 9h4.5M15 9V4.5M15 9l5.25-5.25M15 15h4.5M15 15v4.5m0-4.5l5.25 5.25"
              />
            </svg>
          ) : (
            <svg
              className="w-3 h-3 text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 8V4m0 0h4M4 4l5 5m11-1V4m0 0h-4m4 0l-5 5M4 16v4m0 0h4m-4 0l5-5m11 5l-5-5m5 5v-4m0 4h-4"
              />
            </svg>
          )}
        </button>

        {/* Close button */}
        <button
          type="button"
          onClick={handleClose}
          className="h-full px-3 flex items-center justify-center hover:bg-red-600 hover:text-white transition-colors group"
          title="Close"
        >
          <svg
            className="w-3.5 h-3.5 text-[var(--text-muted)] group-hover:text-white"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}
