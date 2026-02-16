// Archive modal with search and restore functionality
import { useEffect, useState, useRef } from 'react';
import { useAppStore } from '../store';
import type { ArchivedTab } from '../types';

export function ArchiveModal() {
  const {
    archiveModalOpen,
    setArchiveModalOpen,
    archivedTabs,
    archiveSearchResults,
    archivedTabsLoading,
    archiveSearching,
    searchArchive,
    restoreTab,
    deleteArchivedTab,
    activeSpaceId,
  } = useAppStore();

  const [searchQuery, setSearchQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchTimeoutRef = useRef<number | undefined>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const selectedIndexRef = useRef(selectedIndex);

  // Keep ref in sync with state
  useEffect(() => {
    selectedIndexRef.current = selectedIndex;
  }, [selectedIndex]);

  // Focus search input when modal opens
  useEffect(() => {
    if (archiveModalOpen && searchInputRef.current) {
      searchInputRef.current.focus();
      setSelectedIndex(0);
    }
  }, [archiveModalOpen]);

  // Reset search when modal closes
  useEffect(() => {
    if (!archiveModalOpen) {
      setSearchQuery('');
      setSelectedIndex(0);
    }
  }, [archiveModalOpen]);

  // Reset selection when search results change
  useEffect(() => {
    setSelectedIndex(0);
  }, [archivedTabs, archiveSearchResults]);

  // Scroll selected item into view
  useEffect(() => {
    const selectedElement = listRef.current?.querySelector(`[data-index="${selectedIndex}"]`) as HTMLElement;
    if (selectedElement) {
      selectedElement.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  }, [selectedIndex]);

  // Debounced search
  useEffect(() => {
    if (searchTimeoutRef.current) {
      clearTimeout(searchTimeoutRef.current);
    }

    searchTimeoutRef.current = window.setTimeout(() => {
      searchArchive(searchQuery);
    }, 300);

    return () => {
      if (searchTimeoutRef.current) {
        clearTimeout(searchTimeoutRef.current);
      }
    };
  }, [searchQuery, searchArchive]);

  if (!archiveModalOpen) return null;

  // Determine which items to display
  const displayItems: Array<{ tab: ArchivedTab; snippets?: { title?: string; content?: string } }> =
    archiveSearchResults !== null
      ? archiveSearchResults.map((r) => ({
        tab: r.archived_tab,
        snippets: {
          title: r.snippet_title || undefined,
          content: r.snippet_content || undefined,
        },
      }))
      : archivedTabs.map((t) => ({ tab: t }));

  const isLoading = archivedTabsLoading || archiveSearching;

  // Format relative time
  const formatRelativeTime = (dateStr: string): string => {
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

    if (diffDays === 0) return 'Today';
    if (diffDays === 1) return 'Yesterday';
    if (diffDays < 7) return `${diffDays} days ago`;
    if (diffDays < 30) return `${Math.floor(diffDays / 7)} weeks ago`;
    if (diffDays < 365) return `${Math.floor(diffDays / 30)} months ago`;
    return `${Math.floor(diffDays / 365)} years ago`;
  };

  const handleRestore = async (archivedId: string) => {
    // Pass activeSpaceId if available, otherwise backend will restore to original space
    await restoreTab(archivedId, activeSpaceId ?? undefined);
  };

  const handleDelete = async (archivedId: string) => {
    if (confirm('Permanently delete this archived tab? This cannot be undone.')) {
      await deleteArchivedTab(archivedId);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    const maxIndex = displayItems.length - 1;

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
      const item = displayItems[selectedIndexRef.current];
      if (item) {
        handleRestore(item.tab.id);
      }
    } else if (e.key === 'Escape') {
      setArchiveModalOpen(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onClick={() => setArchiveModalOpen(false)}
      onKeyDown={handleKeyDown}
    >
      <div
        className="bg-[var(--bg-primary)] border border-white/10 rounded-xl shadow-2xl w-full max-w-2xl max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-white/5">
          <h2 className="text-lg font-semibold text-[--text-primary]">Archive</h2>
          <button
            onClick={() => setArchiveModalOpen(false)}
            className="p-1 hover:bg-white/5 rounded-lg transition-colors"
          >
            <svg className="w-5 h-5 text-[--text-muted]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Search bar */}
        <div className="px-6 py-4 border-b border-white/5">
          <div className="relative">
            <svg
              className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-[--text-muted]"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
              />
            </svg>
            <input
              ref={searchInputRef}
              type="text"
              placeholder="Search archived tabs..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={handleKeyDown}
              className="w-full pl-10 pr-4 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[--text-primary] placeholder-[--text-muted] focus:outline-none focus:border-[--accent-color] transition-colors"
            />
          </div>
        </div>

        {/* Content */}
        <div ref={listRef} className="flex-1 overflow-y-auto px-6 py-4">
          {isLoading ? (
            <div className="flex items-center justify-center py-12">
              <div className="w-6 h-6 border-2 border-[--accent-color] border-t-transparent rounded-full animate-spin" />
            </div>
          ) : displayItems.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <svg className="w-12 h-12 text-[--text-muted] mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4"
                />
              </svg>
              <p className="text-[--text-muted] text-sm">
                {searchQuery ? 'No matching archived tabs found' : 'No archived tabs yet'}
              </p>
            </div>
          ) : (
            <div className="space-y-2">
              {displayItems.map(({ tab, snippets }, index) => {
                const isSelected = index === selectedIndex;
                return (
                  <div
                    key={tab.id}
                    data-index={index}
                    className={`group relative p-4 rounded-lg transition-colors cursor-pointer ${isSelected
                        ? 'bg-[var(--bg-active)]'
                        : 'hover:bg-[var(--bg-hover)]'
                      }`}
                    onMouseEnter={() => {
                      setSelectedIndex(index);
                    }}
                    onClick={() => handleRestore(tab.id)}
                  >
                    {/* Tab title */}
                    <div className="flex items-start justify-between gap-2 mb-2">
                      <div className="flex-1 min-w-0">
                        <h3
                          className="font-medium text-[--text-primary] truncate"
                          dangerouslySetInnerHTML={{
                            __html: snippets?.title || tab.title,
                          }}
                        />
                      </div>
                      {isSelected && (
                        <div className="flex items-center gap-1">
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              handleRestore(tab.id);
                            }}
                            className="p-1.5 hover:bg-white/10 rounded transition-colors"
                            title="Restore tab"
                          >
                            <svg className="w-4 h-4 text-[--text-secondary]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                              />
                            </svg>
                          </button>
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              handleDelete(tab.id);
                            }}
                            className="p-1.5 hover:bg-red-500/20 rounded transition-colors"
                            title="Delete permanently"
                          >
                            <svg className="w-4 h-4 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                              />
                            </svg>
                          </button>
                        </div>
                      )}
                    </div>

                    {/* Content snippet */}
                    {snippets?.content && (
                      <div
                        className="text-xs text-[--text-muted] mb-2 line-clamp-2"
                        dangerouslySetInnerHTML={{ __html: snippets.content }}
                      />
                    )}

                    {/* Metadata */}
                    <div className="flex items-center gap-3 text-xs text-[--text-muted]">
                      {/* Space name */}
                      <div className="flex items-center gap-1">
                        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
                          />
                        </svg>
                        <span>
                          {tab.space_id ? tab.space_name : `deleted space: ${tab.space_name}`}
                        </span>
                      </div>

                      {/* Database */}
                      {tab.database && (
                        <div className="flex items-center gap-1">
                          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4"
                            />
                          </svg>
                          <span>{tab.database}</span>
                        </div>
                      )}

                      {/* Archived time */}
                      <div className="flex items-center gap-1 ml-auto">
                        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                          />
                        </svg>
                        <span>{formatRelativeTime(tab.archived_at)}</span>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer hint */}
        {!isLoading && displayItems.length > 0 && (
          <div className="px-6 py-3 border-t border-white/5 text-xs text-[--text-muted] text-center">
            Click a tab to restore • Hover for more actions
          </div>
        )}
      </div>
    </div>
  );
}
