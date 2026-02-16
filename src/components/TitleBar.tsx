// Custom window title bar for frameless window (T047)
import { getCurrentWindow } from '@tauri-apps/api/window';
import { GlobalSearch } from './GlobalSearch';
import { useState, useEffect } from 'react';

interface TitleBarProps {
  spaceColor?: string;
  sidebarWidth?: number;
  sidebarHidden?: boolean;
}

export function TitleBar({ spaceColor = '#6366f1', sidebarWidth = 280, sidebarHidden = false }: TitleBarProps) {
  const [isMaximized, setIsMaximized] = useState(false);
  const appWindow = getCurrentWindow();

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

  return (
    <div className="flex items-center h-8 select-none relative" style={{ background: `${spaceColor}40` }}>
      {/* Left section - Above sidebar (transparent to show sidebar gradient) */}
      <div
        className="relative h-full flex items-center overflow-hidden"
        style={{
          width: `${currentSidebarWidth}px`,
          // background handled by parent
        }}
      >
        {/* App title (draggable) - Only show when sidebar is visible */}
        {!sidebarHidden && (
          <div data-tauri-drag-region className="flex items-center gap-2 px-3 h-full relative z-10 whitespace-nowrap">
            <svg
              className="w-3.5 h-3.5 text-[var(--text-muted)] flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"
              />
            </svg>
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              Larik SQL Studio
            </span>
          </div>
        )}
      </div>

      {/* Resize handle separator removed as per request */}

      {/* Right section - Above main content area (draggable) */}
      <div
        data-tauri-drag-region
        className="flex-1 h-full"
      />

      {/* Center Search Bar - Absolute positioned to be centered in window */}
      <div className="absolute inset-0 flex items-center justify-center pointer-events-none z-20">
        <GlobalSearch />
      </div>

      {/* Window controls - Absolute positioned on the right */}
      <div className="absolute right-0 top-0 h-full flex items-center">
        {/* Minimize button */}
        <button
          type="button"
          onClick={handleMinimize}
          className="h-full px-4 flex items-center justify-center hover:bg-white/5 transition-colors group"
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
          className="h-full px-4 flex items-center justify-center hover:bg-white/5 transition-colors group"
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
          className="h-full px-4 flex items-center justify-center hover:bg-red-600 hover:text-white transition-colors group"
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
