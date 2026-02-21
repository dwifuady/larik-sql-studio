// Global keyboard shortcuts hook for Larik SQL Studio (T041)
import { useEffect, useCallback, useState } from 'react';
import { SqlExportDialog } from '../components/SqlExportDialog';
import { useAppStore } from '../store';
import * as api from '../api';

export interface KeyboardShortcut {
  id: string;
  label: string;
  description: string;
  keys: string[];
  category: 'tabs' | 'query' | 'spaces' | 'navigation' | 'general';
  action: () => void | Promise<void>;
  enabled?: boolean;
}

/**
 * Parse a keyboard shortcut string into modifiers and key
 * e.g., "Ctrl+T" -> { ctrl: true, shift: false, alt: false, meta: false, key: "t" }
 */
function parseShortcut(shortcut: string): { ctrl: boolean; shift: boolean; alt: boolean; meta: boolean; key: string } {
  const parts = shortcut.toLowerCase().split('+');
  const key = parts[parts.length - 1];

  return {
    ctrl: parts.includes('ctrl'),
    shift: parts.includes('shift'),
    alt: parts.includes('alt'),
    meta: parts.includes('meta') || parts.includes('cmd'),
    key,
  };
}

/**
 * Check if a keyboard event matches a shortcut string
 */
function matchesShortcut(event: KeyboardEvent, shortcut: string): boolean {
  const parsed = parseShortcut(shortcut);
  const eventKey = event.key.toLowerCase();

  // Handle special keys
  let keyMatches = false;
  if (parsed.key === 'enter') {
    keyMatches = eventKey === 'enter';
  } else if (parsed.key === '\\') {
    keyMatches = eventKey === '\\' || event.code === 'Backslash';
  } else if (parsed.key === 'escape' || parsed.key === 'esc') {
    keyMatches = eventKey === 'escape';
  } else if (parsed.key === '?') {
    keyMatches = eventKey === '?' || (event.shiftKey && eventKey === '/');
  } else if (/^[0-9]$/.test(parsed.key)) {
    keyMatches = eventKey === parsed.key || event.code === `Digit${parsed.key}`;
  } else {
    keyMatches = eventKey === parsed.key;
  }

  return (
    keyMatches &&
    event.ctrlKey === parsed.ctrl &&
    event.shiftKey === parsed.shift &&
    event.altKey === parsed.alt &&
    event.metaKey === parsed.meta
  );
}

/**
 * Format shortcut for display
 */
export function formatShortcut(shortcut: string): string {
  return shortcut
    .split('+')
    .map(part => {
      const lower = part.toLowerCase();
      if (lower === 'ctrl') return 'Ctrl';
      if (lower === 'shift') return '⇧';
      if (lower === 'alt') return 'Alt';
      if (lower === 'meta' || lower === 'cmd') return '⌘';
      if (lower === 'enter') return '↵';
      if (lower === 'escape' || lower === 'esc') return 'Esc';
      if (lower === '\\') return '\\';
      return part.toUpperCase();
    })
    .join(' + ');
}

/**
 * Get all registered shortcuts for display
 */
export function getAllShortcuts(): Array<{
  id: string;
  label: string;
  description: string;
  shortcut: string;
  category: string;
}> {
  return [
    // Tabs
    { id: 'new-tab', label: 'New Tab', description: 'Create a new query tab', shortcut: 'Ctrl+T', category: 'Tabs' },
    { id: 'close-tab', label: 'Close Tab', description: 'Close the current tab', shortcut: 'Ctrl+W', category: 'Tabs' },
    { id: 'next-tab', label: 'Next Tab', description: 'Switch to the next tab', shortcut: 'Ctrl+Tab', category: 'Tabs' },
    { id: 'previous-tab', label: 'Previous Tab', description: 'Switch to the previous tab', shortcut: 'Ctrl+Shift+Tab', category: 'Tabs' },
    { id: 'pin-tab', label: 'Toggle Pin', description: 'Pin or unpin the current tab', shortcut: 'Ctrl+P', category: 'Tabs' },
    { id: 'export-sql-file', label: 'Export as SQL File', description: 'Save the current tab to a .sql file', shortcut: 'Ctrl+Shift+E', category: 'Tabs' },

    // Query
    { id: 'execute-query', label: 'Execute Query', description: 'Execute the query or selection', shortcut: 'Ctrl+Enter', category: 'Query' },
    { id: 'execute-query-f5', label: 'Execute Query', description: 'Execute all queries or the current selection', shortcut: 'F5', category: 'Query' },
    { id: 'execute-to-new-tab', label: 'Execute to New Tab', description: 'Execute query and show results in a new result tab', shortcut: 'Ctrl+\\', category: 'Query' },
    { id: 'force-save', label: 'Force Save', description: 'Manually save the current tab content', shortcut: 'Ctrl+S', category: 'Query' },
    { id: 'format-sql', label: 'Format SQL', description: 'Format the SQL query or selection', shortcut: 'Ctrl+Alt+F', category: 'Query' },
    { id: 'toggle-results', label: 'Toggle Results', description: 'Show or hide the results panel', shortcut: 'Ctrl+R', category: 'Query' },

    // Spaces
    { id: 'switch-space-1', label: 'Switch to Space 1', description: 'Switch to the first space', shortcut: 'Alt+1', category: 'Spaces' },
    { id: 'switch-space-2', label: 'Switch to Space 2', description: 'Switch to the second space', shortcut: 'Alt+2', category: 'Spaces' },
    { id: 'switch-space-3', label: 'Switch to Space 3', description: 'Switch to the third space', shortcut: 'Alt+3', category: 'Spaces' },
    { id: 'switch-space-4', label: 'Switch to Space 4', description: 'Switch to the fourth space', shortcut: 'Alt+4', category: 'Spaces' },
    { id: 'switch-space-5', label: 'Switch to Space 5', description: 'Switch to the fifth space', shortcut: 'Alt+5', category: 'Spaces' },
    { id: 'switch-space-6', label: 'Switch to Space 6', description: 'Switch to the sixth space', shortcut: 'Alt+6', category: 'Spaces' },
    { id: 'switch-space-7', label: 'Switch to Space 7', description: 'Switch to the seventh space', shortcut: 'Alt+7', category: 'Spaces' },
    { id: 'switch-space-8', label: 'Switch to Space 8', description: 'Switch to the eighth space', shortcut: 'Alt+8', category: 'Spaces' },
    { id: 'switch-space-9', label: 'Switch to Space 9', description: 'Switch to the ninth space', shortcut: 'Alt+9', category: 'Spaces' },

    // Navigation
    { id: 'toggle-sidebar', label: 'Toggle Sidebar', description: 'Show or hide the sidebar', shortcut: 'Ctrl+Shift+S', category: 'Navigation' },
    { id: 'command-palette', label: 'Command Palette', description: 'Open the command palette', shortcut: 'Ctrl+Shift+P', category: 'Navigation' },
    { id: 'shortcuts-help', label: 'Keyboard Shortcuts', description: 'Show keyboard shortcuts help', shortcut: 'Ctrl+?', category: 'Navigation' },
  ];
}

/**
 * Hook for managing global keyboard shortcuts
 */
export function useKeyboardShortcuts() {
  const {
    deleteTab,
    toggleTabPinned,
    autosaveContent,
    setActiveSpace,
    setCommandPaletteOpen,
    getActiveTab,
    // getActiveSpace,
    spaces,
    // activeSpaceId,
    executeQueryAppend,
    nextTab,
    previousTab,
  } = useAppStore();

  const setShortcutsDialogOpen = useAppStore((state) => state.setShortcutsDialogOpen);

  // Export Dialog State
  const [isExportDialogOpen, setIsExportDialogOpen] = useState(false);
  const [exportHasNotes, setExportHasNotes] = useState(false);

  // Toggle sidebar handler
  const handleToggleSidebar = useCallback(() => {
    useAppStore.getState().toggleSidebarHidden();
  }, []);

  // Create new tab handler
  const handleNewTab = useCallback(async () => {
    const { setNewTabSelectorOpen, spaces, activeSpaceId, tabs, createTab } = useAppStore.getState();
    const activeSpace = spaces.find(s => s.id === activeSpaceId);
    if (activeSpace?.database_type?.toLowerCase() === 'sqlite') {
      const tabCount = tabs.length + 1;
      await createTab(`Query ${tabCount}`, 'query', '');
      setNewTabSelectorOpen(false); // Just in case
    } else {
      setNewTabSelectorOpen(true);
    }
  }, []);

  // Close current tab handler
  const handleCloseTab = useCallback(async () => {
    const activeTab = getActiveTab();
    if (activeTab && !activeTab.is_pinned) {
      await deleteTab(activeTab.id);
    }
  }, [deleteTab, getActiveTab]);

  // Toggle pin handler
  const handleTogglePin = useCallback(async () => {
    const activeTab = getActiveTab();
    if (activeTab) {
      await toggleTabPinned(activeTab.id);
    }
  }, [toggleTabPinned, getActiveTab]);

  // Force save handler
  const handleForceSave = useCallback(async () => {
    const activeTab = getActiveTab();
    if (activeTab && activeTab.content) {
      await autosaveContent(activeTab.id, activeTab.content);
    }
  }, [autosaveContent, getActiveTab]);

  // Execute query to new result tab handler (appends results instead of replacing)
  const handleExecuteToNewTab = useCallback(async () => {
    const activeTab = getActiveTab();
    if (activeTab && activeTab.content) {
      // Use executeQueryAppend to append results instead of replacing
      // This uses the full content - QueryEditor will override with selection via its own action
      await executeQueryAppend(activeTab.id, activeTab.content, null);
      // The new results are automatically appended and the active index is set to the first new result
    }
  }, [executeQueryAppend, getActiveTab]);

  // Switch space handler
  const handleSwitchSpace = useCallback(async (index: number) => {
    if (index >= 0 && index < spaces.length) {
      await setActiveSpace(spaces[index].id);
    }
  }, [setActiveSpace, spaces]);

  // Show shortcuts dialog
  const handleShowShortcuts = useCallback(() => {
    setShortcutsDialogOpen?.(true);
  }, [setShortcutsDialogOpen]);



  // Handle export confirmation — user chose include/exclude notes, now save the file
  const handleExportConfirm = useCallback(async (includeNotes: boolean) => {
    const activeTab = getActiveTab();
    const { addToast } = useAppStore.getState();

    if (!activeTab) return;

    try {
      // Show OS Save As dialog AFTER the user chose their export option
      const defaultFilename = api.sanitizeFilename(activeTab.title);
      const filePath = await api.saveSqlFileDialog(defaultFilename);
      if (!filePath) return; // User cancelled Save As

      let contentToExport = undefined;
      if (!includeNotes && activeTab.content) {
        const { removeNotes } = await import('../utils/noteManager');
        contentToExport = removeNotes(activeTab.content);
      }

      await api.exportTabAsSql(activeTab.id, filePath, contentToExport);

      const filename = filePath.split(/[/\\]/).pop() || 'file';
      addToast({
        type: 'success',
        message: `Exported to ${filename}`,
        duration: 3000
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      addToast({
        type: 'error',
        message: `Failed to export: ${message}`,
        duration: 5000
      });
    } finally {
      setIsExportDialogOpen(false);
    }
  }, [getActiveTab]);

  // Export SQL file handler — show the export options dialog first
  const handleExportSqlFile = useCallback(() => {
    const activeTab = getActiveTab();
    const { addToast } = useAppStore.getState();

    if (!activeTab) {
      addToast({
        type: 'error',
        message: 'No active tab to export',
        duration: 3000
      });
      return;
    }

    const hasNotes = !!(activeTab.content && activeTab.content.includes('-- @note: '));
    setExportHasNotes(hasNotes);
    setIsExportDialogOpen(true);
  }, [getActiveTab]);

  // Toggle results visibility (Ctrl+R)
  const handleToggleResultsVisibility = useCallback(() => {
    const activeTab = getActiveTab();
    if (activeTab) {
      useAppStore.getState().toggleResultsHidden(activeTab.id);
    }
  }, [getActiveTab]);

  // Global keyboard event handler
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      // Don't trigger shortcuts when typing in inputs (except for specific ones)
      const target = event.target as HTMLElement;
      const isInput = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA';
      const isMonacoEditor = target.closest('.monaco-editor') !== null;

      // Ctrl+Shift+P: Command Palette (works everywhere)
      if (matchesShortcut(event, 'Ctrl+Shift+P')) {
        event.preventDefault();
        setCommandPaletteOpen(true);
        return;
      }

      // Ctrl+Shift+S: Toggle sidebar (works everywhere)
      if (matchesShortcut(event, 'Ctrl+Shift+S')) {
        event.preventDefault();
        handleToggleSidebar();
        return;
      }

      // Ctrl+Shift+E: Export as SQL file (works everywhere)
      if (matchesShortcut(event, 'Ctrl+Shift+E')) {
        event.preventDefault();
        handleExportSqlFile();
        return;
      }

      // Ctrl+?: Show shortcuts help (works everywhere)
      if (matchesShortcut(event, 'Ctrl+Shift+?') || (event.ctrlKey && event.key === '?')) {
        event.preventDefault();
        handleShowShortcuts();
        return;
      }

      // Alt+1-9: Switch spaces (works everywhere)
      for (let i = 1; i <= 9; i++) {
        if (matchesShortcut(event, `Alt+${i}`)) {
          event.preventDefault();
          handleSwitchSpace(i - 1);
          return;
        }
      }

      // Skip remaining shortcuts if in a regular input (not Monaco)
      if (isInput && !isMonacoEditor) {
        return;
      }

      // Ctrl+R: Toggle results visibility (works everywhere to override browser refresh)
      if (matchesShortcut(event, 'Ctrl+R')) {
        event.preventDefault();
        handleToggleResultsVisibility();
        return;
      }

      // Ctrl+T: New tab (only if spaces exist)
      if (matchesShortcut(event, 'Ctrl+T')) {
        if (spaces.length > 0) {
          event.preventDefault();
          handleNewTab();
        }
        return;
      }

      // Ctrl+W: Close tab (only if not pinned)
      if (matchesShortcut(event, 'Ctrl+W')) {
        event.preventDefault();
        handleCloseTab();
        return;
      }

      // Ctrl+Tab: Next tab (T045)
      if (event.ctrlKey && event.key === 'Tab' && !event.shiftKey) {
        event.preventDefault();
        nextTab();
        return;
      }

      // Ctrl+Shift+Tab: Previous tab (T045)
      if (event.ctrlKey && event.shiftKey && event.key === 'Tab') {
        event.preventDefault();
        previousTab();
        return;
      }

      // Ctrl+P: Toggle pin (conflicts with Monaco find, so check if not focused there)
      if (matchesShortcut(event, 'Ctrl+P') && !isMonacoEditor) {
        event.preventDefault();
        handleTogglePin();
        return;
      }

      // Ctrl+S: Force save
      if (matchesShortcut(event, 'Ctrl+S')) {
        event.preventDefault();
        handleForceSave();
        return;
      }

      // Ctrl+\: Execute to new result tab
      // Only handle if NOT in Monaco editor - let Monaco's action handle it with smart statement detection
      if (matchesShortcut(event, 'Ctrl+\\') && !isMonacoEditor) {
        event.preventDefault();
        handleExecuteToNewTab();
        return;
      }

      // F5: Execute query (prevent browser refresh, let Monaco editor handle via its action)
      if (matchesShortcut(event, 'F5')) {
        event.preventDefault();
        // The action will be handled by Monaco editor if in editor context
        return;
      }
    };

    // Add listener at document level to capture all events
    document.addEventListener('keydown', handleKeyDown, { capture: true });

    return () => {
      document.removeEventListener('keydown', handleKeyDown, { capture: true });
    };
  }, [
    handleNewTab,
    handleCloseTab,
    handleTogglePin,
    handleToggleSidebar,
    handleForceSave,
    handleExecuteToNewTab,
    handleSwitchSpace,
    handleShowShortcuts,
    handleExportSqlFile,
    handleToggleResultsVisibility,
    setCommandPaletteOpen,
    nextTab,
    previousTab,
  ]);

  const ExportDialogComponent = (
    <SqlExportDialog
      isOpen={isExportDialogOpen}
      onClose={() => {
        setIsExportDialogOpen(false);
      }}
      onConfirm={handleExportConfirm}
      hasNotes={exportHasNotes}
    />
  );

  return { exportDialog: ExportDialogComponent };
}

export default useKeyboardShortcuts;
