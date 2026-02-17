// Arc-style main application layout with browser-like design
import { useEffect, useRef, useState, useCallback } from 'react';
import { useAppStore } from '../store';
import { LayoutList, Database, Archive, Settings } from 'lucide-react';
import { DatabaseExplorer } from './DatabaseExplorer/DatabaseExplorer';
import { SpacesSelector } from './SpacesSelector';
import { TabsList } from './TabsList';
import { QueryEditor } from './QueryEditor';
import { CommandPalette } from './CommandPalette';
import { ShortcutsDialog } from './ShortcutsDialog';
import { SnippetsDialog } from './SnippetsDialog';
import { SettingsDialog } from './SettingsDialog';
import { TitleBar } from './TitleBar';
import { ToastContainer } from './Toast';
import { ArchiveModal } from './ArchiveModal';
import { ArchiveSection } from './ArchiveSection';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import { spaceHasConnection } from '../types';
import * as api from '../api';
import { getCurrentWindow } from '@tauri-apps/api/window';

export function AppLayout() {
  // Atomic selectors to prevent re-renders
  const loadSpaces = useAppStore(s => s.loadSpaces);
  const loadSnippets = useAppStore(s => s.loadSnippets);
  const loadAppSettings = useAppStore(s => s.loadAppSettings);
  const loadAutoArchiveSettings = useAppStore(s => s.loadAutoArchiveSettings);
  const archivedTabsCount = useAppStore(s => s.archivedTabsCount);
  const setArchiveModalOpen = useAppStore(s => s.setArchiveModalOpen);

  const sidebarWidth = useAppStore(s => s.sidebarWidth);
  const setSidebarWidth = useAppStore(s => s.setSidebarWidth);
  const sidebarHidden = useAppStore(s => s.sidebarHidden);
  const sidebarHoveredWhenHidden = useAppStore(s => s.sidebarHoveredWhenHidden);
  const setSidebarHoveredWhenHidden = useAppStore(s => s.setSidebarHoveredWhenHidden);
  const sidebarView = useAppStore(s => s.sidebarView);
  const setSidebarView = useAppStore(s => s.setSidebarView);

  const activeTabId = useAppStore(s => s.activeTabId);
  const tabs = useAppStore(s => s.tabs);
  const spaces = useAppStore(s => s.spaces);
  const activeSpaceId = useAppStore(s => s.activeSpaceId);
  const spacesLoading = useAppStore(s => s.spacesLoading);

  const spaceConnectionStatus = useAppStore(s => s.spaceConnectionStatus);
  const isConnecting = useAppStore(s => s.isConnecting);
  const connectToSpace = useAppStore(s => s.connectToSpace);
  const disconnectFromSpace = useAppStore(s => s.disconnectFromSpace);

  const spaceDatabases = useAppStore(s => s.spaceDatabases);
  const databasesLoading = useAppStore(s => s.databasesLoading);

  const createTab = useAppStore(s => s.createTab);
  const updateTabDatabase = useAppStore(s => s.updateTabDatabase);

  const newTabSelectorOpen = useAppStore(s => s.newTabSelectorOpen);
  const setNewTabSelectorOpen = useAppStore(s => s.setNewTabSelectorOpen);

  const commandPaletteOpen = useAppStore(s => s.commandPaletteOpen);
  const setCommandPaletteOpen = useAppStore(s => s.setCommandPaletteOpen);

  const shortcutsDialogOpen = useAppStore(s => s.shortcutsDialogOpen);
  const setShortcutsDialogOpen = useAppStore(s => s.setShortcutsDialogOpen);

  const snippetsDialogOpen = useAppStore(s => s.snippetsDialogOpen);
  const setSnippetsDialogOpen = useAppStore(s => s.setSnippetsDialogOpen);

  const settingsDialogOpen = useAppStore(s => s.settingsDialogOpen);
  const setSettingsDialogOpen = useAppStore(s => s.setSettingsDialogOpen);

  const setCreateSpaceModalOpen = useAppStore(s => s.setCreateSpaceModalOpen);

  const theme = useAppStore(s => s.theme);
  const toggleTheme = useAppStore(s => s.toggleTheme);
  const initTheme = useAppStore(s => s.initTheme);

  // Shallow compare for addToast to avoid re-renders? 
  // Actually addToast is stable function from zustand if defined correctly, 
  // but here we are selecting specific properties.
  // The above pattern is repetitive but ensures we only subscribe to what we use.

  // Initialize global keyboard shortcuts (T041)
  const { exportDialog } = useKeyboardShortcuts();

  const [isResizing, setIsResizing] = useState(false);
  const [isDbDropdownOpen, setIsDbDropdownOpen] = useState(false);
  const [selectedDbIndex, setSelectedDbIndex] = useState(0);
  const [isDraggingFile, setIsDraggingFile] = useState(false);
  const [dbSearchQuery, setDbSearchQuery] = useState('');
  const sidebarRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const newTabDropdownRef = useRef<HTMLDivElement>(null);
  const dbSearchInputRef = useRef<HTMLInputElement>(null);
  const fileDropProcessingRef = useRef(false);
  const lastDropTimestampRef = useRef(0);

  // Get active space for theming and info
  const activeSpace = spaces.find(s => s.id === activeSpaceId);
  const spaceColor = activeSpace?.color || '#6366f1';
  const hasConnection = activeSpace ? spaceHasConnection(activeSpace) : false;
  const isConnected = spaceConnectionStatus?.is_connected ?? false;

  // Load spaces on mount (non-blocking)
  useEffect(() => {
    console.log('[AppLayout] Starting initialization...');

    // Hide the HTML loader immediately when React renders
    const htmlLoader = document.getElementById('html-loader');
    if (htmlLoader) {
      htmlLoader.classList.add('hidden');
    }

    // Load app settings first (validation, etc.), then load spaces
    // loadSpaces will restore the last workspace/tab if available
    Promise.all([loadAppSettings(), loadAutoArchiveSettings()]).then(() => {
      console.log('[AppLayout] App settings loaded');
      // Load spaces (non-blocking - component has its own loading state)
      return loadSpaces();
    }).then(() => {
      console.log('[AppLayout] Spaces loaded');
    }).catch(err => {
      console.error('[AppLayout] Failed to load:', err);
    });
  }, []); // Empty dependency array - only run once on mount

  // Defer snippets loading to idle time (not needed for initial render)
  useEffect(() => {
    let cancelled = false;

    const loadSnippetsDeferred = () => {
      if (cancelled) return;
      console.log('[AppLayout] Loading snippets (deferred)...');
      loadSnippets().then(() => {
        console.log('[AppLayout] Snippets loaded');
      }).catch(err => {
        console.error('[AppLayout] Failed to load snippets:', err);
      });
    };

    // Use requestIdleCallback if available, otherwise setTimeout
    if ('requestIdleCallback' in window) {
      const idleId = window.requestIdleCallback(loadSnippetsDeferred, { timeout: 2000 });
      return () => {
        cancelled = true;
        window.cancelIdleCallback(idleId);
      };
    } else {
      const timeoutId = setTimeout(loadSnippetsDeferred, 500);
      return () => {
        cancelled = true;
        clearTimeout(timeoutId);
      };
    }
  }, [loadSnippets]);

  // Initialize theme on mount
  useEffect(() => {
    initTheme();
  }, [initTheme]);

  // Listen for system theme changes when theme is set to 'system'
  useEffect(() => {
    if (theme !== 'system') return;

    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = (e: MediaQueryListEvent) => {
      const root = document.documentElement;
      root.setAttribute('data-theme', e.matches ? 'dark' : 'light');
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, [theme]);

  // Note: Global keyboard shortcuts (Ctrl+Shift+P, Ctrl+T, etc.) are now handled by useKeyboardShortcuts hook (T041)

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsDbDropdownOpen(false);
      }
    };
    if (isDbDropdownOpen) {
      document.addEventListener('mousedown', handleClickOutside);
    }
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [isDbDropdownOpen]);

  // Close new tab selector when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (newTabDropdownRef.current && !newTabDropdownRef.current.contains(e.target as Node)) {
        setNewTabSelectorOpen(false);
      }
    };
    if (newTabSelectorOpen) {
      document.addEventListener('mousedown', handleClickOutside);
    }
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [newTabSelectorOpen, setNewTabSelectorOpen]);

  // Filter databases based on search query
  const filteredDatabases = spaceDatabases.filter(db =>
    db.toLowerCase().includes(dbSearchQuery.toLowerCase())
  );

  // Check if default DB matches search
  const defaultDbName = activeSpace?.connection_database || 'Default DB';
  const showDefaultDb = defaultDbName.toLowerCase().includes(dbSearchQuery.toLowerCase());

  // Handle keyboard navigation for database selector
  useEffect(() => {
    if (!newTabSelectorOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      // Calculate total options based on filtered results
      const totalOptions = isConnected && (showDefaultDb || filteredDatabases.length > 0)
        ? (showDefaultDb ? 1 : 0) + filteredDatabases.length
        : 1;

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedDbIndex((prev) => (prev + 1) % totalOptions);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedDbIndex((prev) => (prev - 1 + totalOptions) % totalOptions);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        // Get the selected database from filtered list
        if (isConnected && (showDefaultDb || filteredDatabases.length > 0)) {
          if (showDefaultDb && selectedDbIndex === 0) {
            handleCreateNewTab(activeSpace?.connection_database || null);
          } else {
            const dbIndex = showDefaultDb ? selectedDbIndex - 1 : selectedDbIndex;
            handleCreateNewTab(filteredDatabases[dbIndex]);
          }
        } else {
          handleCreateNewTab(null);
        }
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setNewTabSelectorOpen(false);
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [newTabSelectorOpen, selectedDbIndex, isConnected, filteredDatabases, showDefaultDb, activeSpace]);

  // Reset selected index, search query, and focus when opening/closing
  useEffect(() => {
    if (newTabSelectorOpen) {
      setSelectedDbIndex(0);
      setDbSearchQuery('');
      // Focus the search input so user can start typing immediately
      setTimeout(() => {
        if (dbSearchInputRef.current) {
          dbSearchInputRef.current.focus();
        }
      }, 50);
    }
  }, [newTabSelectorOpen]);

  // Reset selected index when search query changes
  useEffect(() => {
    setSelectedDbIndex(0);
  }, [dbSearchQuery]);

  // Close new tab selector when clicking outside (original effect)

  // Handle database selection
  const handleSelectDatabase = (db: string | null) => {
    if (activeTab) {
      updateTabDatabase(activeTab.id, db);
    }
    setIsDbDropdownOpen(false);
  };

  // Handle new tab creation with database selection
  const handleCreateNewTab = async (db: string | null) => {
    const tabCount = tabs.length + 1;
    const newTab = await createTab(`Query ${tabCount}`, 'query', '');
    if (newTab && db !== null) {
      await updateTabDatabase(newTab.id, db);
    }
    setNewTabSelectorOpen(false);
  };

  // Handle sidebar resize
  const startResize = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  }, []);

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isResizing) return;
      const newWidth = Math.max(220, Math.min(400, e.clientX));
      setSidebarWidth(newWidth);
    };

    const handleMouseUp = () => {
      setIsResizing(false);
    };

    if (isResizing) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [isResizing, setSidebarWidth]);

  const activeTab = tabs.find(t => t.id === activeTabId);

  // Calculate sidebar width considering hidden state
  const currentSidebarWidth = sidebarHidden && !sidebarHoveredWhenHidden ? 0 : sidebarWidth;
  const sidebarOpacity = sidebarHidden && !sidebarHoveredWhenHidden ? 0 : 1;
  const isShowingSidebarHover = sidebarHidden && sidebarHoveredWhenHidden;

  // Handle mouse enter on left edge when sidebar is hidden
  const handleLeftEdgeMouseEnter = (e: React.MouseEvent) => {
    if (sidebarHidden && e.clientX < 20) {
      setSidebarHoveredWhenHidden(true);
    }
  };

  // Handle mouse leave from sidebar when it's in hover state
  const handleSidebarMouseLeave = () => {
    if (isShowingSidebarHover) {
      setSidebarHoveredWhenHidden(false);
    }
  };

  // Tauri file drop event listener
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const handleFileDrop = async (paths: string[]) => {
      const now = Date.now();

      // Prevent duplicate drops within 500ms
      if (fileDropProcessingRef.current || (now - lastDropTimestampRef.current) < 500) {
        console.log('[FileDrop] Ignoring duplicate drop event - already processing or too soon');
        return;
      }

      fileDropProcessingRef.current = true;
      lastDropTimestampRef.current = now;
      console.log('[FileDrop] Files dropped:', paths);

      const { addToast } = useAppStore.getState();
      const currentActiveSpaceId = useAppStore.getState().activeSpaceId;

      if (!currentActiveSpaceId) {
        addToast({
          type: 'error',
          message: 'No active space selected',
          duration: 3000
        });
        return;
      }

      const sqlFiles = paths.filter(path => path.toLowerCase().endsWith('.sql'));

      if (sqlFiles.length === 0) {
        addToast({
          type: 'error',
          message: 'No SQL files found. Please drop .sql files.',
          duration: 3000
        });
        return;
      }

      // Import all dropped SQL files
      let successCount = 0;
      let errorCount = 0;
      let lastImportedTab = null;

      for (const filePath of sqlFiles) {
        try {
          console.log('[FileDrop] Importing:', filePath);
          const tab = await api.importSqlFileAsTab(currentActiveSpaceId, filePath);
          lastImportedTab = tab;

          // Add tab to store
          useAppStore.setState((state) => ({
            tabs: [tab, ...state.tabs],
          }));

          successCount++;
        } catch (error) {
          console.error('[FileDrop] Failed to import file:', filePath, error);
          errorCount++;
        }
      }

      // Set last imported tab as active
      if (lastImportedTab) {
        useAppStore.setState({ activeTabId: lastImportedTab.id });
      }

      // Show summary toast
      if (successCount > 0) {
        const fileName = sqlFiles[successCount - 1].split(/[/\\]/).pop() || 'file';
        const message = successCount === 1
          ? `Imported ${fileName}`
          : `Imported ${successCount} SQL file${successCount > 1 ? 's' : ''}`;

        addToast({
          type: 'success',
          message,
          duration: 3000
        });
      }

      if (errorCount > 0) {
        addToast({
          type: 'error',
          message: `Failed to import ${errorCount} file${errorCount > 1 ? 's' : ''}`,
          duration: 5000
        });
      }

      // Reset processing flag
      fileDropProcessingRef.current = false;
    };

    const setupListeners = async () => {
      try {
        const appWindow = getCurrentWindow();
        console.log('[FileDrop] Setting up file drop listeners on window');

        // Listen for file drop events
        unlisten = await appWindow.onDragDropEvent((event) => {
          console.log('[FileDrop] Drag event:', event);

          if (event.payload.type === 'enter') {
            console.log('[FileDrop] Drag enter');
            setIsDraggingFile(true);
          } else if (event.payload.type === 'drop') {
            console.log('[FileDrop] Drop with paths:', event.payload.paths);
            setIsDraggingFile(false);
            handleFileDrop(event.payload.paths);
          } else {
            // Handle leave and other events
            console.log('[FileDrop] Drag leave/other');
            setIsDraggingFile(false);
          }
        });

        console.log('[FileDrop] Listener registered successfully');
      } catch (error) {
        console.error('[FileDrop] Error setting up listeners:', error);
      }
    };

    setupListeners();

    return () => {
      if (unlisten) {
        console.log('[FileDrop] Cleaning up listener');
        unlisten();
      }
    };
  }, []);

  return (
    <div className="flex flex-col h-full w-full overflow-hidden bg-[var(--bg-primary)] relative">
      {/* Arc-style gradient background that spans entire window */}
      <div
        className="absolute inset-0 pointer-events-none"
        style={{
          background: `linear-gradient(135deg, ${spaceColor}08 0%, ${spaceColor}04 25%, transparent 50%, ${spaceColor}04 75%, ${spaceColor}08 100%)`,
          zIndex: 1,
        }}
      />

      {/* Drag-and-drop overlay */}
      {isDraggingFile && (
        <div className="fixed inset-0 z-[100] pointer-events-none">
          <div className="absolute inset-0 bg-blue-500/10 backdrop-blur-sm" />
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="bg-[var(--bg-secondary)] border-2 border-dashed border-blue-400 rounded-2xl px-12 py-8 shadow-2xl">
              <div className="flex flex-col items-center gap-4">
                <svg className="w-16 h-16 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
                </svg>
                <div className="text-center">
                  <p className="text-lg font-semibold text-[var(--text-primary)]">Drop SQL files to import</p>
                  <p className="text-sm text-[var(--text-muted)] mt-1">Files will be imported as new tabs</p>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}



      {/* Custom title bar (T047) */}
      <div className="relative z-10">
        <TitleBar spaceColor={spaceColor} sidebarWidth={sidebarWidth} sidebarHidden={sidebarHidden} />
      </div>

      {/* Main content area with sidebar */}
      <div
        className="flex flex-1 overflow-hidden relative z-10"
        onMouseMove={handleLeftEdgeMouseEnter}
      >
        {/* Hover zone indicator when sidebar is hidden */}
        {sidebarHidden && !sidebarHoveredWhenHidden && (
          <div
            className="absolute left-0 top-0 bottom-0 w-1 hover:w-2 bg-transparent hover:bg-[var(--accent-color)]/20 transition-all z-40 cursor-pointer"
            onMouseEnter={() => setSidebarHoveredWhenHidden(true)}
            style={{ width: '20px' }}
          />
        )}

        {/* Sidebar with browser-like layout - overlay when hidden */}
        <aside
          ref={sidebarRef}
          className={`flex flex-col overflow-hidden transition-all duration-200 ease-out backdrop-blur ${isShowingSidebarHover ? 'rounded-2xl m-2 mt-2' : ''}`}
          style={{
            width: isShowingSidebarHover ? sidebarWidth - 16 : (sidebarHidden ? sidebarWidth : currentSidebarWidth),
            minWidth: isShowingSidebarHover ? sidebarWidth - 16 : (sidebarHidden ? sidebarWidth : currentSidebarWidth),
            height: isShowingSidebarHover ? 'calc(100% - 16px)' : '100%',
            opacity: sidebarOpacity,
            background: isShowingSidebarHover
              ? `linear-gradient(${spaceColor}40, ${spaceColor}40), var(--bg-primary)` // Soft tint (25%) over opaque background
              : `${spaceColor}40`, // 40 = ~25% opacity
            position: sidebarHidden ? 'absolute' : 'relative',
            left: 0,
            top: 0,
            zIndex: sidebarHidden ? (isShowingSidebarHover ? 30 : -1) : 10,
            boxShadow: isShowingSidebarHover ? '0 4px 24px rgba(0,0,0,0.12), 0 0 0 1px var(--border-color)' : 'none',
            pointerEvents: sidebarOpacity === 0 ? 'none' : 'auto',
          }}
          onMouseLeave={handleSidebarMouseLeave}
        >

          {/* Content */}
          <div className="relative z-10 flex flex-col flex-1 min-h-0">
            {/* URL Bar style database selector at top */}
            <div className="px-3 pt-4 pb-2">
              {spacesLoading && spaces.length === 0 ? (
                /* Skeleton for database bar */
                <div className="flex items-center gap-2 px-3 py-2.5 bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-xl">
                  <div className="w-4 h-4 rounded bg-[var(--bg-active)] animate-pulse" />
                  <div className="flex-1 h-4 rounded bg-[var(--bg-active)] animate-pulse" />
                </div>
              ) : (
                <div
                  className="text-xs flex items-center gap-2 px-3 py-2.5 bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-xl hover:bg-[var(--bg-active)] hover:border-[var(--border-color)] transition-all group"
                  style={{
                    boxShadow: isConnected ? `0 0 0 1px ${spaceColor}10` : 'none'
                  }}
                >
                  <svg
                    className="w-4 h-4 flex-shrink-0 transition-colors"
                    style={{ color: isConnected ? spaceColor : 'var(--text-muted)' }}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4" />
                  </svg>
                  {isConnected && spaceDatabases.length > 0 ? (
                    <div ref={dropdownRef} className="flex-1 relative min-w-0">
                      {/* Custom dropdown trigger */}
                      <button
                        onClick={() => !databasesLoading && setIsDbDropdownOpen(!isDbDropdownOpen)}
                        disabled={databasesLoading}
                        className="w-full flex items-center justify-between gap-2 text-left disabled:opacity-50"
                      >
                        <span className="text-xs font-medium text-[var(--text-primary)] flex-1 overflow-hidden relative group/marquee max-w-full">
                          <span
                            key={activeTab?.database || activeSpace?.connection_database}
                            ref={(el) => {
                              if (el && el.parentElement) {
                                const parent = el.parentElement;
                                // Need a tiny delay for layout to settle
                                setTimeout(() => {
                                  const parentWidth = parent.offsetWidth;
                                  const scrollWidth = el.scrollWidth;
                                  if (scrollWidth > parentWidth) {
                                    parent.style.setProperty('--marquee-width', `${parentWidth}px`);
                                    const duration = Math.max(3, scrollWidth / 30);
                                    parent.style.setProperty('--marquee-duration', `${duration}s`);
                                    parent.classList.add('animate-marquee-hover');
                                  } else {
                                    parent.classList.remove('animate-marquee-hover');
                                  }
                                }, 50);
                              }
                            }}
                            className="marquee-content inline-block whitespace-nowrap"
                          >
                            {activeTab?.database || activeSpace?.connection_database || 'Default DB'}
                          </span>
                        </span>
                      </button>

                      {/* Dropdown menu */}
                      {isDbDropdownOpen && (
                        <div
                          className="absolute top-full mt-2 z-50 py-1 rounded-lg border border-[var(--border-color)] shadow-xl overflow-hidden animate-in fade-in slide-in-from-top-2 duration-150 bg-[var(--bg-secondary)]"
                          style={{
                            backdropFilter: 'blur(20px)',
                            left: '-35px',
                            right: '-35px',
                            maxHeight: '280px',
                            overflowY: 'auto',
                          }}
                        >
                          {/* Default database option */}
                          <button
                            onClick={() => handleSelectDatabase(null)}
                            className={`w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors ${!activeTab?.database
                              ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                              : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                              }`}
                          >
                            <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 3v4M3 5h4M6 17v4m-2-2h4m5-16l2.286 6.857L21 12l-5.714 2.143L13 21l-2.286-6.857L5 12l5.714-2.143L13 3z" />
                            </svg>
                            <span className="truncate">{activeSpace?.connection_database || 'Default DB'}</span>
                            {!activeTab?.database && (
                              <svg className="w-4 h-4 ml-auto flex-shrink-0 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                              </svg>
                            )}
                          </button>

                          {/* Separator */}
                          <div className="my-1 mx-2 border-t border-[var(--border-color)]" />

                          {/* Database list */}
                          {spaceDatabases.map((db) => (
                            <button
                              key={db}
                              onClick={() => handleSelectDatabase(db)}
                              className={`w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors ${activeTab?.database === db
                                ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                                : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                                }`}
                            >
                              <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                              </svg>
                              <span className="truncate">{db}</span>
                              {activeTab?.database === db && (
                                <svg className="w-4 h-4 ml-auto flex-shrink-0 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                                </svg>
                              )}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  ) : (
                    <span className="flex-1 text-sm text-[var(--text-muted)] truncate">
                      {hasConnection ? (isConnected ? activeSpace?.connection_database : 'Not connected') : 'No database'}
                    </span>
                  )}
                  <svg
                    className={`w-4 h-4 flex-shrink-0 pointer-events-none transition-all ${isConnected && spaceDatabases.length > 0 ? 'opacity-60 group-hover:opacity-100' : 'opacity-40'
                      } ${isDbDropdownOpen ? 'rotate-180' : ''}`}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                  </svg>
                </div>
              )}
            </div>

            {/* Space Name & Connection Controls - Compact Header */}
            {activeSpace && (
              <div className="px-3 py-2 pb-0 flex items-center justify-between group">
                <span className="text-xs font-semibold text-[var(--text-primary)] truncate" title={activeSpace.name}>
                  {activeSpace.name}
                </span>

                {hasConnection && (
                  <div className="flex items-center gap-2">
                    {/* Status Dot */}
                    <div
                      className={`w-1.5 h-1.5 rounded-full transition-colors ${isConnected ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.4)]' : 'bg-[var(--text-muted)]'}`}
                      title={isConnected ? 'Connected' : 'Disconnected'}
                    />

                    {/* Connect/Disconnect Button - Only visible on hover or if disconnected */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        isConnected ? disconnectFromSpace() : connectToSpace();
                      }}
                      disabled={isConnecting}
                      className={`p-1 rounded hover:bg-[var(--bg-hover)] transition-all ${isConnected
                        ? 'text-[var(--text-muted)] hover:text-red-400 opacity-0 group-hover:opacity-100'
                        : 'text-[var(--text-muted)] hover:text-green-400 opacity-100'
                        }`}
                      title={isConnected ? 'Disconnect' : 'Connect'}
                    >
                      <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
                      </svg>
                    </button>
                  </div>
                )}
              </div>
            )}

            {/* Sidebar View Toggle & Actions */}
            <div className="px-2 py-1.5 flex items-center gap-1.5">
              <div className="bg-black/5 dark:bg-white/5 p-1 rounded-lg flex gap-1 flex-1">
                <button
                  onClick={() => setSidebarView('tabs')}
                  className={`flex-1 flex items-center justify-center py-1 rounded-md text-[11px] font-medium transition-all duration-200 ${sidebarView === 'tabs'
                    ? 'bg-white dark:bg-[#2d2d2d] shadow-sm'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-secondary)]'
                    }`}
                  style={sidebarView === 'tabs' ? { color: activeSpace?.color || 'var(--accent-color)' } : undefined}
                >
                  <LayoutList className="w-3 h-3 mr-1.5" />
                  Tabs
                </button>
                <button
                  onClick={() => setSidebarView('explorer')}
                  className={`flex-1 flex items-center justify-center py-1 rounded-md text-[11px] font-medium transition-all duration-200 ${sidebarView === 'explorer'
                    ? 'bg-white dark:bg-[#2d2d2d] shadow-sm'
                    : 'text-[var(--text-muted)] hover:text-[var(--text-secondary)]'
                    }`}
                  style={sidebarView === 'explorer' ? { color: activeSpace?.color || 'var(--accent-color)' } : undefined}
                >
                  <Database className="w-3 h-3 mr-1.5" />
                  Explorer
                </button>
              </div>

              {/* New Tab Button - Inline */}
              <button
                onClick={() => setNewTabSelectorOpen(true)}
                className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-black/5 dark:hover:bg-white/5 text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors"
                title="New Tab (Ctrl+T)"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
              </button>
            </div>

            {/* Tabs list OR Database Explorer fills remaining space */}
            {sidebarView === 'tabs' ? (
              <TabsList onNewTabClick={() => setNewTabSelectorOpen(true)} hasSpaces={spaces.length > 0} />
            ) : (
              <div className="flex-1 overflow-hidden min-h-0">
                <DatabaseExplorer />
              </div>
            )}


          </div>

          {/* Footer Section: Archive | Spaces | Settings */}
          <div className="px-3 py-2 border-t border-[var(--border-subtle)] flex items-center justify-between shrink-0">
            {/* Left: Archive */}
            <button
              onClick={() => setArchiveModalOpen(true)}
              className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors relative"
              title="Archive"
            >
              <Archive className="w-4 h-4" />
              {archivedTabsCount > 0 && (
                <span className="absolute top-2 right-2 w-1.5 h-1.5 bg-[var(--accent-color)] rounded-full" />
              )}
            </button>

            {/* Center: Spaces Selector */}
            <SpacesSelector />

            {/* Right: Settings (Command Palette) */}
            <button
              onClick={() => setCommandPaletteOpen(true)}
              className="w-8 h-8 flex items-center justify-center rounded-lg hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors"
              title="Settings & Commands (Ctrl+Shift+P)"
            >
              <Settings className="w-4 h-4" />
            </button>
          </div>
        </aside>

        {/* Resize handle - thin and subtle (only show when sidebar is visible and not hidden) */}
        {!sidebarHidden && (
          <div
            className={`w-px cursor-col-resize transition-colors flex-shrink-0 ${isResizing ? 'bg-[var(--accent-color)]' : 'bg-[var(--border-color)] hover:bg-[var(--text-muted)]'
              }`}
            onMouseDown={startResize}
          />
        )}

        {/* Main content area */}
        <main className="flex-1 flex flex-col min-w-0 relative z-10 transition-colors duration-200">
          {spaces.length === 0 ? (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <button
                  onClick={() => setCreateSpaceModalOpen(true)}
                  className="w-16 h-16 rounded-2xl bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] flex items-center justify-center mx-auto mb-4 transition-colors group"
                >
                  <svg className="w-8 h-8 text-[var(--text-muted)] group-hover:text-[var(--text-secondary)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 4v16m8-8H4" />
                  </svg>
                </button>
                <p className="text-[var(--text-secondary)] mb-1">Welcome to Larik SQL Studio</p>
                <button
                  onClick={() => setCreateSpaceModalOpen(true)}
                  className="text-[var(--accent-color)] hover:text-[var(--accent-color)]/80 transition-colors font-medium"
                >
                  Create a space to get started
                </button>
              </div>
            </div>
          ) : activeTab ? (
            <QueryEditor tab={activeTab} />
          ) : (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <div className="w-16 h-16 rounded-2xl bg-[var(--bg-hover)] flex items-center justify-center mx-auto mb-4">
                  <svg className="w-8 h-8 text-[var(--text-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                  </svg>
                </div>
                <p className="text-[var(--text-secondary)] mb-1">No query selected</p>
                <p className="text-[var(--text-muted)] text-sm">Select a tab or create a new query</p>
              </div>
            </div>
          )}
        </main>

        {/* Command Palette (Ctrl+Shift+P) */}
        <CommandPalette
          isOpen={commandPaletteOpen}
          onClose={() => setCommandPaletteOpen(false)}
        />

        {/* Keyboard Shortcuts Help Dialog (Ctrl+?) */}
        <ShortcutsDialog
          isOpen={shortcutsDialogOpen}
          onClose={() => setShortcutsDialogOpen(false)}
        />

        {/* SQL Snippets Management Dialog */}
        <SnippetsDialog
          isOpen={snippetsDialogOpen}
          onClose={() => setSnippetsDialogOpen(false)}
        />

        {/* Application Settings Dialog */}
        <SettingsDialog
          isOpen={settingsDialogOpen}
          onClose={() => setSettingsDialogOpen(false)}
        />

        {/* SQL Export Dialog (from hook) */}
        {exportDialog}

        {/* New Tab Database Selector Modal - at root level for proper centering */}
        {newTabSelectorOpen && (
          <div
            ref={newTabDropdownRef}
            tabIndex={-1}
            className="fixed inset-0 flex items-center justify-center pointer-events-none outline-none"
            style={{ zIndex: 50 }}
          >
            <div
              className="bg-black/40 absolute inset-0 pointer-events-auto"
              onClick={() => setNewTabSelectorOpen(false)}
            />
            <div
              className="relative pointer-events-auto w-96 py-1 rounded-lg border border-[var(--border-color)] shadow-xl overflow-hidden animate-in fade-in zoom-in-95 duration-150 bg-[var(--bg-secondary)]"
              style={{
                backdropFilter: 'blur(20px)',
                maxHeight: '380px',
                overflowY: 'auto',
                zIndex: 51,
              }}
            >
              <div className="px-3 py-2 text-[10px] font-semibold text-[var(--text-muted)] uppercase tracking-wider">
                Select database for new tab
              </div>

              {/* Search input */}
              {isConnected && spaceDatabases.length > 0 && (
                <div className="px-3 pb-2">
                  <div className="relative">
                    <svg className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-[var(--text-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                    </svg>
                    <input
                      ref={dbSearchInputRef}
                      type="text"
                      value={dbSearchQuery}
                      onChange={(e) => setDbSearchQuery(e.target.value)}
                      placeholder="Search databases..."
                      className="w-full pl-9 pr-8 py-1.5 text-sm bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-md text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:border-[var(--accent-color)] focus:bg-[var(--bg-active)]"
                    />
                    {dbSearchQuery && (
                      <button
                        type="button"
                        onClick={() => setDbSearchQuery('')}
                        className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--text-muted)] hover:text-[var(--text-secondary)]"
                      >
                        <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                        </svg>
                      </button>
                    )}
                  </div>
                </div>
              )}

              {isConnected && spaceDatabases.length > 0 ? (
                <>
                  {/* Default database option - only show if matches search */}
                  {showDefaultDb && (
                    <button
                      type="button"
                      onClick={() => handleCreateNewTab(activeSpace?.connection_database || null)}
                      onMouseEnter={() => setSelectedDbIndex(0)}
                      className={`w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors border-0 ${selectedDbIndex === 0
                        ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                        : 'bg-[var(--bg-hover)] text-[var(--text-primary)] hover:bg-[var(--bg-active)]'
                        }`}
                    >
                      <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 3v4M3 5h4M6 17v4m-2-2h4m5-16l2.286 6.857L21 12l-5.714 2.143L13 21l-2.286-6.857L5 12l5.714-2.143L13 3z" />
                      </svg>
                      <span className="truncate">{activeSpace?.connection_database || 'Default DB'}</span>
                      <svg className="w-4 h-4 ml-auto flex-shrink-0 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                      </svg>
                    </button>
                  )}

                  {/* Separator */}
                  {showDefaultDb && filteredDatabases.length > 0 && <div className="my-1 mx-2 border-t border-[var(--border-color)]" />}

                  {/* Filtered database list */}
                  {filteredDatabases.map((db, index) => (
                    <button
                      type="button"
                      key={db}
                      onClick={() => handleCreateNewTab(db)}
                      onMouseEnter={() => setSelectedDbIndex(showDefaultDb ? index + 1 : index)}
                      className={`w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors border-0 ${selectedDbIndex === (showDefaultDb ? index + 1 : index)
                        ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                        : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                        }`}
                    >
                      <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                      </svg>
                      <span className="truncate">{db}</span>
                    </button>
                  ))}

                  {/* No results message */}
                  {!showDefaultDb && filteredDatabases.length === 0 && dbSearchQuery && (
                    <div className="px-3 py-4 text-center text-sm text-[var(--text-muted)]">
                      No databases matching "{dbSearchQuery}"
                    </div>
                  )}
                </>
              ) : hasConnection && !isConnected ? (
                /* Show connect option when server is not connected */
                <div className="px-3 py-2">
                  <div className="flex items-center gap-2 mb-3 text-[var(--text-muted)]">
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                    </svg>
                    <span className="text-sm">Server not connected</span>
                  </div>
                  <button
                    type="button"
                    onClick={async () => {
                      await connectToSpace();
                    }}
                    disabled={isConnecting}
                    className="w-full px-3 py-2.5 text-sm flex items-center justify-center gap-2 transition-colors border-0 rounded-lg bg-green-500/20 hover:bg-green-500/30 text-green-400 disabled:opacity-50"
                  >
                    {isConnecting ? (
                      <>
                        <svg className="w-4 h-4 animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                        </svg>
                        <span>Connecting...</span>
                      </>
                    ) : (
                      <>
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
                        </svg>
                        <span>Connect to {activeSpace?.connection_host || 'server'}</span>
                      </>
                    )}
                  </button>
                  <div className="mt-3 border-t border-[var(--border-color)] pt-3">
                    <button
                      type="button"
                      onClick={() => handleCreateNewTab(null)}
                      className="w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors border-0 text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-secondary)]"
                    >
                      <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 3v4M3 5h4M6 17v4m-2-2h4m5-16l2.286 6.857L21 12l-5.714 2.143L13 21l-2.286-6.857L5 12l5.714-2.143L13 3z" />
                      </svg>
                      <span className="truncate">Create tab without connection</span>
                    </button>
                  </div>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => handleCreateNewTab(null)}
                  className="w-full px-3 py-2 text-left text-sm flex items-center gap-2 transition-colors border-0 text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                >
                  <svg className="w-3.5 h-3.5 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 3v4M3 5h4M6 17v4m-2-2h4m5-16l2.286 6.857L21 12l-5.714 2.143L13 21l-2.286-6.857L5 12l5.714-2.143L13 3z" />
                  </svg>
                  <span className="truncate">Create with default database</span>
                </button>
              )}
            </div>
          </div>
        )}
      </div>

      {/* Toast notifications */}
      <ToastContainer />

      {/* Archive modal */}
      <ArchiveModal />
    </div>
  );
}
