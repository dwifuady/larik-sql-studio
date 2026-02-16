// Command Palette (Ctrl+Shift+P) for quick access to all actions
import { useEffect, useState, useRef, useMemo, useCallback } from 'react';
import { useAppStore } from '../store';
import * as api from '../api';

export interface Command {
  id: string;
  label: string;
  description?: string;
  category: string;
  keywords?: string[];
  action: () => void | Promise<void>;
  icon?: React.ReactNode;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
}

// Simple fuzzy search implementation
function fuzzyMatch(text: string, search: string): { matches: boolean; score: number } {
  const textLower = text.toLowerCase();
  const searchLower = search.toLowerCase();

  if (searchLower === '') return { matches: true, score: 0 };
  if (textLower.includes(searchLower)) {
    // Exact substring match gets high score
    return { matches: true, score: 100 - textLower.indexOf(searchLower) };
  }

  // Check for fuzzy match (letters in order)
  let searchIndex = 0;
  let lastMatchIndex = -1;
  let score = 0;

  for (let i = 0; i < textLower.length && searchIndex < searchLower.length; i++) {
    if (textLower[i] === searchLower[searchIndex]) {
      // Bonus for consecutive matches
      if (lastMatchIndex === i - 1) {
        score += 5;
      }
      lastMatchIndex = i;
      searchIndex++;
      score += 1;
    }
  }

  if (searchIndex === searchLower.length) {
    return { matches: true, score };
  }

  return { matches: false, score: 0 };
}

export function CommandPalette({ isOpen, onClose }: CommandPaletteProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [recentCommands, setRecentCommands] = useState<string[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const selectedIndexRef = useRef(selectedIndex);
  const filteredCommandsRef = useRef<Command[]>([]);

  // Keep ref in sync with state
  useEffect(() => {
    selectedIndexRef.current = selectedIndex;
  }, [selectedIndex]);

  const {
    createTab,
    deleteTab,
    toggleTabPinned,
    connectToSpace,
    disconnectFromSpace,
    refreshSchema,
    setCreateSpaceModalOpen,
    setShortcutsDialogOpen,
    setSnippetsDialogOpen,
    getActiveTab,
    isConnected,
    clearQueryResult,
    theme,
    setTheme,
    activeSpaceId,
    addToast,
    toggleSidebarHidden,
    setSettingsDialogOpen,
    setNewTabSelectorOpen,
  } = useAppStore();

  // Define all available commands
  const allCommands = useMemo((): Command[] => {
    const activeTab = getActiveTab();
    const connected = isConnected();

    return [
      // Tab commands
      {
        id: 'new-tab',
        label: 'New Query Tab',
        description: 'Create a new query tab',
        category: 'Tabs',
        keywords: ['create', 'add', 'query'],
        action: () => {
          setNewTabSelectorOpen(true);
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
        ),
      },
      {
        id: 'close-tab',
        label: 'Close Current Tab',
        description: 'Close the currently active tab',
        category: 'Tabs',
        keywords: ['delete', 'remove'],
        action: async () => {
          if (activeTab) {
            await deleteTab(activeTab.id);
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        ),
      },
      {
        id: 'pin-tab',
        label: activeTab?.is_pinned ? 'Unpin Current Tab' : 'Pin Current Tab',
        description: 'Toggle pin status of the current tab',
        category: 'Tabs',
        keywords: ['pin', 'unpin', 'sticky'],
        action: async () => {
          if (activeTab) {
            await toggleTabPinned(activeTab.id);
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
          </svg>
        ),
      },
      {
        id: 'clear-results',
        label: 'Clear Query Results',
        description: 'Clear the results of the current query',
        category: 'Query',
        keywords: ['reset', 'remove'],
        action: () => {
          if (activeTab) {
            clearQueryResult(activeTab.id);
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
          </svg>
        ),
      },
      {
        id: 'export-sql-file',
        label: 'Export Current Tab as SQL File',
        description: 'Save the current tab content to a .sql file',
        category: 'Tabs',
        keywords: ['save', 'download', 'file', 'sql', 'export'],
        action: async () => {
          if (!activeTab) {
            addToast({
              type: 'error',
              message: 'No active tab to export',
              duration: 3000
            });
            onClose();
            return;
          }

          try {
            const defaultFilename = api.sanitizeFilename(activeTab.title);
            const filePath = await api.saveSqlFileDialog(defaultFilename);
            if (!filePath) {
              onClose();
              return; // User cancelled
            }

            // Strip sticky notes from exported content
            let contentToExport = undefined;
            if (activeTab.content?.includes('-- @note: ')) {
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
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
          </svg>
        ),
      },
      {
        id: 'import-sql-file',
        label: 'Import SQL File as Tab',
        description: 'Open a .sql file as a new tab',
        category: 'Tabs',
        keywords: ['open', 'load', 'file', 'sql', 'import'],
        action: async () => {
          if (!activeSpaceId) {
            addToast({
              type: 'error',
              message: 'No active space selected',
              duration: 3000
            });
            onClose();
            return;
          }

          try {
            const filePath = await api.openSqlFileDialog();
            if (!filePath) {
              onClose();
              return; // User cancelled
            }

            const tab = await api.importSqlFileAsTab(activeSpaceId, filePath);

            // Manually add the tab to the store and set it as active
            useAppStore.setState((state) => ({
              tabs: [tab, ...state.tabs],
              activeTabId: tab.id
            }));

            const filename = filePath.split(/[/\\]/).pop() || 'file';
            addToast({
              type: 'success',
              message: `Imported ${filename}`,
              duration: 3000
            });
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            addToast({
              type: 'error',
              message: `Failed to import: ${message}`,
              duration: 5000
            });
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
          </svg>
        ),
      },

      // Space commands
      {
        id: 'new-space',
        label: 'New Space',
        description: 'Create a new workspace',
        category: 'Spaces',
        keywords: ['create', 'add', 'workspace'],
        action: () => {
          setCreateSpaceModalOpen(true);
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
          </svg>
        ),
      },

      // Connection commands
      {
        id: 'connect',
        label: connected ? 'Disconnect from Database' : 'Connect to Database',
        description: connected ? 'Disconnect from the current space database' : 'Connect to the current space database',
        category: 'Connection',
        keywords: ['database', 'server'],
        action: async () => {
          if (connected) {
            await disconnectFromSpace();
          } else {
            await connectToSpace();
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
          </svg>
        ),
      },
      {
        id: 'refresh-schema',
        label: 'Refresh Schema',
        description: 'Reload database schema for autocompletion',
        category: 'Connection',
        keywords: ['reload', 'schema', 'metadata'],
        action: async () => {
          if (connected) {
            await refreshSchema();
          }
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
        ),
      },

      // Theme commands
      {
        id: 'theme-dark',
        label: 'Theme: Dark',
        description: 'Switch to dark theme',
        category: 'Appearance',
        keywords: ['dark', 'mode', 'theme', 'color'],
        action: () => {
          setTheme('dark');
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z" />
          </svg>
        ),
      },
      {
        id: 'theme-light',
        label: 'Theme: Light',
        description: 'Switch to light theme',
        category: 'Appearance',
        keywords: ['light', 'mode', 'theme', 'color', 'bright'],
        action: () => {
          setTheme('light');
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z" />
          </svg>
        ),
      },
      {
        id: 'theme-system',
        label: 'Theme: System',
        description: 'Follow system color scheme',
        category: 'Appearance',
        keywords: ['system', 'auto', 'mode', 'theme', 'color', 'preference'],
        action: () => {
          setTheme('system');
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
        ),
      },

      {
        id: 'open-settings',
        label: 'Open Settings',
        description: 'Configure application preferences',
        category: 'Appearance',
        keywords: ['settings', 'preferences', 'config', 'options', 'sticky', 'notes', 'performance'],
        action: () => {
          setSettingsDialogOpen(true);
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        ),
      },

      // Help commands
      {
        id: 'keyboard-shortcuts',
        label: 'Keyboard Shortcuts',
        description: 'Show all keyboard shortcuts',
        category: 'Help',
        keywords: ['keyboard', 'shortcuts', 'hotkeys', 'keybindings', 'help'],
        action: () => {
          setShortcutsDialogOpen(true);
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 11c0 3.517-1.009 6.799-2.753 9.571m-3.44-2.04l.054-.09A13.916 13.916 0 008 11a4 4 0 118 0c0 1.017-.07 2.019-.203 3m-2.118 6.844A21.88 21.88 0 0015.171 17m3.839 1.132c.645-2.266.99-4.659.99-7.132A8 8 0 008 4.07M3 15.364c.64-1.319 1-2.8 1-4.364 0-1.457.39-2.823 1.07-4" />
          </svg>
        ),
      },
      {
        id: 'manage-snippets',
        label: 'Manage SQL Snippets',
        description: 'View, create, edit, and import SQL code snippets',
        category: 'Editor',
        keywords: ['snippets', 'templates', 'code', 'autocomplete', 'abbreviations', 'dbeaver', 'import'],
        action: () => {
          setSnippetsDialogOpen(true);
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
          </svg>
        ),
      },

      // Navigation commands
      {
        id: 'toggle-sidebar',
        label: 'Toggle Sidebar',
        description: 'Show or hide the sidebar',
        category: 'Navigation',
        keywords: ['sidebar', 'panel', 'view', 'visibility', 'toggle'],
        action: () => {
          toggleSidebarHidden();
          onClose();
        },
        icon: (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
          </svg>
        ),
      },
    ];
  }, [
    createTab,
    deleteTab,
    toggleTabPinned,
    connectToSpace,
    disconnectFromSpace,
    refreshSchema,
    setCreateSpaceModalOpen,
    setShortcutsDialogOpen,
    setSnippetsDialogOpen,
    getActiveTab,
    isConnected,
    clearQueryResult,
    theme,
    setTheme,
    setTheme,
    toggleSidebarHidden,
    setSettingsDialogOpen,
    setNewTabSelectorOpen,
    onClose,
  ]);

  // Filter and sort commands based on search query
  const filteredCommands = useMemo(() => {
    let result: Command[];
    if (!searchQuery.trim()) {
      // Show recent commands first when no search
      const recent = allCommands.filter(cmd => recentCommands.includes(cmd.id));
      const others = allCommands.filter(cmd => !recentCommands.includes(cmd.id));
      result = [...recent, ...others];
    } else {
      // Search in label, description, category, and keywords
      result = allCommands
        .map(cmd => {
          const labelMatch = fuzzyMatch(cmd.label, searchQuery);
          const descMatch = cmd.description ? fuzzyMatch(cmd.description, searchQuery) : { matches: false, score: 0 };
          const categoryMatch = fuzzyMatch(cmd.category, searchQuery);
          const keywordsMatch = cmd.keywords
            ? Math.max(...cmd.keywords.map(kw => fuzzyMatch(kw, searchQuery).score), 0)
            : 0;

          const matches = labelMatch.matches || descMatch.matches || categoryMatch.matches || keywordsMatch > 0;
          const score = Math.max(labelMatch.score, descMatch.score, categoryMatch.score, keywordsMatch);

          return { cmd, matches, score };
        })
        .filter(r => r.matches)
        .sort((a, b) => b.score - a.score)
        .map(r => r.cmd);
    }

    filteredCommandsRef.current = result;
    return result;
  }, [allCommands, searchQuery, recentCommands]);

  // Reset state when opened
  useEffect(() => {
    if (isOpen) {
      setSearchQuery('');
      setSelectedIndex(0);
      selectedIndexRef.current = 0;
      // Load recent commands from localStorage
      const stored = localStorage.getItem('recentCommands');
      if (stored) {
        try {
          setRecentCommands(JSON.parse(stored));
        } catch (e) {
          console.error('Failed to parse recent commands:', e);
        }
      }
      // Focus input after a short delay
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [isOpen]);

  // Execute command and track in recent
  const executeCommand = useCallback(async (command: Command | undefined) => {
    if (!command) return;

    try {
      await command.action();

      // Add to recent commands (keep last 5)
      setRecentCommands(prev => {
        const updated = [command.id, ...prev.filter(id => id !== command.id)].slice(0, 5);
        localStorage.setItem('recentCommands', JSON.stringify(updated));
        return updated;
      });
    } catch (error) {
      console.error('Command execution failed:', error);
    }
  }, []);

  // Scroll selected item into view
  useEffect(() => {
    if (listRef.current) {
      const selectedElement = listRef.current.querySelector(`[data-index="${selectedIndex}"]`) as HTMLElement;
      if (selectedElement) {
        selectedElement.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    }
  }, [selectedIndex]);

  if (!isOpen) return null;

  const handleKeyDown = (e: React.KeyboardEvent) => {
    const commands = filteredCommandsRef.current;
    const maxIndex = commands.length - 1;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      const newIndex = Math.min(selectedIndexRef.current + 1, maxIndex);
      selectedIndexRef.current = newIndex;
      setSelectedIndex(newIndex);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      const newIndex = Math.max(selectedIndexRef.current - 1, 0);
      selectedIndexRef.current = newIndex;
      setSelectedIndex(newIndex);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      const command = commands[selectedIndexRef.current];
      if (command) {
        executeCommand(command);
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[20vh] bg-black/50 backdrop-blur-sm animate-in fade-in duration-150"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        className="w-full max-w-2xl mx-4 rounded-xl border border-[var(--border-color)] shadow-2xl overflow-hidden animate-in slide-in-from-top-4 duration-200 bg-[var(--bg-secondary)]"
        style={{
          backdropFilter: 'blur(20px)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Search input */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-[var(--border-color)]">
          <svg className="w-5 h-5 text-[var(--text-muted)] flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            ref={inputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              setSelectedIndex(0);
              selectedIndexRef.current = 0;
            }}
            onKeyDown={handleKeyDown}
            placeholder="Type a command or search..."
            className="w-full pl-3 pr-8 py-1.5 text-sm bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-md text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:border-[var(--accent-color)] focus:bg-[var(--bg-active)]"
            autoFocus
          />
          <kbd className="px-2 py-1 text-xs text-[var(--text-muted)] bg-[var(--bg-hover)] rounded border border-[var(--border-color)]">ESC</kbd>
        </div>

        {/* Command list */}
        <div ref={listRef} className="max-h-[400px] overflow-y-auto py-2">
          {filteredCommands.length === 0 ? (
            <div className="px-4 py-8 text-center text-[var(--text-muted)]">
              No commands found
            </div>
          ) : (
            <>
              {!searchQuery && recentCommands.length > 0 && (
                <div className="px-4 py-1 text-xs font-medium text-[var(--text-muted)] uppercase tracking-wider">
                  Recent
                </div>
              )}
              {filteredCommands.map((command, index) => {
                const showCategoryHeader = index === 0 || filteredCommands[index - 1].category !== command.category;
                const nextIsNotRecent = !searchQuery && index === recentCommands.length - 1;
                const isSelected = index === selectedIndex;

                return (
                  <div
                    key={command.id}
                    data-index={index}
                    className={`${isSelected ? 'bg-[var(--bg-active)]' : ''}`}
                  >
                    {showCategoryHeader && searchQuery && (
                      <div className="px-4 py-1 mt-2 text-xs font-medium text-[var(--text-muted)] uppercase tracking-wider">
                        {command.category}
                      </div>
                    )}
                    {nextIsNotRecent && (
                      <div className="px-4 py-1 mt-2 text-xs font-medium text-[var(--text-muted)] uppercase tracking-wider">
                        All Commands
                      </div>
                    )}
                    <button
                      onClick={() => executeCommand(command)}
                      onMouseEnter={() => {
                        setSelectedIndex(index);
                        selectedIndexRef.current = index;
                      }}
                      className={`w-full px-4 py-2.5 flex items-center gap-3 transition-colors ${isSelected
                        ? 'text-[var(--text-primary)]'
                        : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]'
                        }`}
                    >
                      {command.icon && (
                        <div className="flex-shrink-0 w-8 h-8 rounded-lg bg-[var(--bg-hover)] flex items-center justify-center">
                          {command.icon}
                        </div>
                      )}
                      <div className="flex-1 text-left min-w-0">
                        <div className="font-medium truncate">{command.label}</div>
                        {command.description && (
                          <div className="text-sm text-[var(--text-muted)] truncate">{command.description}</div>
                        )}
                      </div>
                      <div className="flex-shrink-0 text-xs text-[var(--text-muted)]">
                        {command.category}
                      </div>
                    </button>
                  </div>
                );
              })}
            </>
          )}
        </div>

        {/* Footer with hint */}
        <div className="px-4 py-2 border-t border-[var(--border-color)] flex items-center justify-between text-xs text-[var(--text-muted)]">
          <div className="flex items-center gap-4">
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 bg-[var(--bg-hover)] rounded border border-[var(--border-color)]">↑↓</kbd>
              Navigate
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 bg-[var(--bg-hover)] rounded border border-[var(--border-color)]">↵</kbd>
              Execute
            </span>
          </div>
          <span>{filteredCommands.length} {filteredCommands.length === 1 ? 'command' : 'commands'}</span>
        </div>
      </div>
    </div>
  );
}
