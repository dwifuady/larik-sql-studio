
// Arc-style Tabs list with pinned section and browser-style new tab
import { useState, useRef, useEffect, memo, useCallback } from 'react';
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd';
import {
  DndContext,
  DragOverlay,
  useSensor,
  useSensors,
  PointerSensor,
  closestCorners,
  DragStartEvent,
  DragOverEvent,
  DragEndEvent,
  useDraggable,
  useDroppable,
} from '@dnd-kit/core';
import { useAppStore } from '../store';
import { ContextMenu } from './ContextMenu';
import { FolderItem } from './FolderItem';
import { CreateFolderDialog } from './CreateFolderDialog';
import type { Tab, TabFolder } from '../types';
import * as api from '../api';

// Tab item component - defined outside TabsList to prevent recreation on each render
const TabItem = memo(({
  tab,
  isPinned,
  isActive,
  onRename,
  onDelete,
  shouldStartRename,
  onContextMenu,
  onRenameStarted,
  onSetActive,
  spaceColor,
  hasOpenTransaction
}: {
  tab: Tab;
  isPinned: boolean;
  isActive: boolean;
  onRename: (id: string, newTitle: string) => void;
  onDelete: (id: string) => void;
  shouldStartRename?: boolean;
  onContextMenu?: (e: React.MouseEvent, tab: Tab) => void;
  onRenameStarted?: () => void;
  onSetActive: (id: string) => void;
  spaceColor: string;
  hasOpenTransaction?: boolean;
}) => {
  const [isRenaming, setIsRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus input when starting rename
  useEffect(() => {
    if (isRenaming && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [isRenaming]);

  // Trigger rename from parent (e.g., context menu)
  useEffect(() => {
    if (shouldStartRename && !isRenaming) {
      setIsRenaming(true);
      setRenameValue(tab.title);
      onRenameStarted?.();
    }
  }, [shouldStartRename, isRenaming, tab.title, onRenameStarted]);

  const handleStartRename = () => {
    setIsRenaming(true);
    setRenameValue(tab.title);
  };

  const handleFinishRename = () => {
    if (renameValue.trim() && renameValue !== tab.title) {
      onRename(tab.id, renameValue.trim());
    }
    setIsRenaming(false);
    setRenameValue('');
  };

  const handleCancelRename = () => {
    setIsRenaming(false);
    setRenameValue('');
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`${isPinned ? 'Pinned tab' : 'Tab'}: ${tab.title}${tab.database ? ` (${tab.database})` : ''}`}
      aria-current={isActive ? 'page' : undefined}
      className={`
        group flex items-center gap-2 px-2 py-1.5 mx-1 my-0.5 rounded-lg cursor-pointer relative
        transition-all duration-150 ease-out focus:outline-none focus:ring-2 focus:ring-[var(--accent-color)] focus:ring-offset-1
        ${isActive
          ? 'text-[var(--text-primary)]' // Active: background handled by style
          : 'hover:bg-black/5 dark:hover:bg-white/5 text-[var(--text-secondary)]'
        }
      `}
      style={{
        backgroundColor: isActive ? `color-mix(in srgb, ${spaceColor}, transparent 85%)` : undefined
      }}
      onClick={() => onSetActive(tab.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          onSetActive(tab.id);
        } else if (e.key === 'F2') {
          e.preventDefault();
          handleStartRename();
        } else if (e.key === 'Delete' && !isPinned) {
          e.preventDefault();
          onDelete(tab.id);
        }
      }}
      onDoubleClick={handleStartRename}
      onMouseDown={(e) => {
        // Middle click to close tab
        if (e.button === 1) {
          e.preventDefault();
          e.stopPropagation();
          onDelete(tab.id);
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu?.(e, tab);
      }}
    >
      {/* Active colored strip - visible when active */}
      {isActive && (
        <div
          className="absolute left-0 top-1/2 -translate-y-1/2 h-5 w-1 rounded-r-full"
          style={{
            backgroundColor: spaceColor,
            boxShadow: `0 0 8px ${spaceColor}66` // Add 40% opacity glow
          }}
        />
      )}

      {/* Tab icon */}
      <div className="shrink-0" style={{ color: isActive ? spaceColor : 'var(--text-muted)' }}>
        {tab.tab_type === 'query' ? (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
          </svg>
        ) : (
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
          </svg>
        )}
      </div>

      {/* Tab name */}
      {isRenaming ? (
        <input
          ref={inputRef}
          type="text"
          value={renameValue}
          onChange={(e) => setRenameValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') handleFinishRename();
            if (e.key === 'Escape') handleCancelRename();
          }}
          onBlur={handleFinishRename}
          onClick={(e) => e.stopPropagation()}
          className="flex-1 min-w-0 text-sm bg-white/10 px-2 py-0.5 rounded outline-none"
        />
      ) : (
        <div className="flex-1 min-w-0 flex flex-col">
          <span className={`truncate text-sm ${isActive ? 'text-[var(--text-primary)] font-medium' : 'text-[--text-secondary]'}`}>
            {tab.title}
          </span>
          {(tab.database || hasOpenTransaction) && (
            <span className={`truncate text-[10px] leading-tight ${hasOpenTransaction ? 'text-amber-500 font-medium' : 'text-[--text-muted]'}`}>
              {hasOpenTransaction ? '⚠️ Uncommitted Tx' : tab.database}
            </span>
          )}
        </div>
      )}

      {/* Pin indicator for pinned tabs */}
      {isPinned && !isRenaming && (
        <svg className="w-3 h-3 text-[--text-muted] shrink-0" fill="currentColor" viewBox="0 0 24 24">
          <path d="M16 4l4 4-6 6 2 6-2 2-3-5-4 4v2h-2l-1-3-3-1v-2l4-4-5-3 2-2 6 2 6-6z" />
        </svg>
      )}

      {/* Close button */}
      {!isPinned && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDelete(tab.id);
          }}
          className="p-0.5 opacity-0 group-hover:opacity-100 hover:bg-[var(--bg-active)] rounded transition-all shrink-0"
          title="Close"
        >
          <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </div>
  );
}, (prevProps, nextProps) => {
  // Custom comparison to prevent unnecessary re-renders
  // Return true = props are equal = skip re-render
  // Return false = props changed = do re-render

  // Different tab = must re-render
  if (prevProps.tab.id !== nextProps.tab.id) return false;

  // Active status changed = must re-render
  if (prevProps.isActive !== nextProps.isActive) return false;

  // Pinned status changed = must re-render
  if (prevProps.isPinned !== nextProps.isPinned) return false;
  if (prevProps.tab.is_pinned !== nextProps.tab.is_pinned) return false;

  // Title changed = must re-render
  if (prevProps.tab.title !== nextProps.tab.title) return false;

  // Database changed = must re-render (for subtitle)
  if (prevProps.tab.database !== nextProps.tab.database) return false;

  // Rename trigger changed = must re-render
  if (prevProps.shouldStartRename !== nextProps.shouldStartRename) return false;

  // Space color changed = must re-render
  if (prevProps.spaceColor !== nextProps.spaceColor) return false;

  // Transaction state changed = must re-render
  if (prevProps.hasOpenTransaction !== nextProps.hasOpenTransaction) return false;

  // All other changes = skip re-render (handlers are stable)
  return true;
});

// Draggable wrapper for pinned tabs
const DraggableTabItem = memo(({
  tab,
  isOver,
  ...tabItemProps
}: {
  tab: Tab;
  isOver: boolean;
} & React.ComponentProps<typeof TabItem>) => {
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({
    id: `pinned-tab-${tab.id}`,
    data: { type: 'tab', tab },
  });

  const { setNodeRef: setDropRef } = useDroppable({
    id: `pinned-tab-${tab.id}`,
    data: { type: 'tab', tab },
  });

  const style: React.CSSProperties = {
    transform: transform ? `translate3d(${transform.x}px, ${transform.y}px, 0)` : undefined,
    opacity: isDragging ? 0.5 : 1,
    outline: isOver ? '2px solid var(--accent-color)' : undefined,
    outlineOffset: isOver ? '2px' : undefined,
    backgroundColor: isOver ? 'rgba(59, 130, 246, 0.1)' : undefined,
    borderRadius: '8px',
    transition: 'background-color 0.15s ease',
  };

  return (
    <div ref={(node) => { setNodeRef(node); setDropRef(node); }} style={style} {...attributes} {...listeners}>
      <TabItem tab={tab} {...tabItemProps} />
    </div>
  );
});

// Draggable wrapper for folders
const DraggableFolder = memo(({
  folder,
  tabs,
  isOver,
  ...folderItemProps
}: {
  folder: TabFolder;
  tabs: Tab[];
  isOver: boolean;
  spaceColor: string;
} & Omit<React.ComponentProps<typeof FolderItem>, 'folder' | 'tabs' | 'spaceColor'>) => {
  const { attributes, listeners, setNodeRef: setDragRef } = useDraggable({
    id: `folder-${folder.id}`,
    data: { type: 'folder', folder },
  });

  const { setNodeRef: setDropRef } = useDroppable({
    id: `folder-header-${folder.id}`,
    data: { type: 'folder-header', folder },
  });

  const style: React.CSSProperties = {
    outline: isOver ? '2px solid var(--accent-color)' : undefined,
    outlineOffset: isOver ? '2px' : undefined,
    backgroundColor: isOver ? 'rgba(59, 130, 246, 0.1)' : undefined,
    borderRadius: '8px',
    transition: 'background-color 0.15s ease',
  };

  return (
    <div ref={setDropRef} style={style}>
      <FolderItem
        folder={folder}
        tabs={tabs}
        dragHandleRef={setDragRef}
        dragHandleAttributes={attributes}
        dragHandleListeners={listeners}
        {...folderItemProps}
      />
    </div>
  );
});

// Droppable zone for ungrouped area (to remove tabs from folders)
const UngroupedDropZone = memo(({ isOver, children }: { isOver: boolean; children: React.ReactNode }) => {
  const { setNodeRef } = useDroppable({
    id: 'drop-ungrouped',
    data: { type: 'ungrouped' },
  });

  const style: React.CSSProperties = {
    outline: isOver ? '2px dashed var(--accent-color)' : undefined,
    outlineOffset: isOver ? '4px' : undefined,
    backgroundColor: isOver ? 'var(--accent-color-10)' : undefined,
    borderRadius: '8px',
    transition: 'all 0.2s ease',
  };

  return (
    <div ref={setNodeRef} style={style}>
      {children}
    </div>
  );
});

export function TabsList({ onNewTabClick }: { onNewTabClick?: () => void }) {
  const {
    tabs,
    activeTabId,
    tabsLoading,
    activeSpaceId,
    setActiveTab,
    updateTab,
    deleteTab,
    toggleTabPinned,
    reorderTabs,
    createFolder,
    createFolderFromTabs,
    updateFolder,
    deleteFolder,
    toggleFolderExpanded,
    addTabToFolder,
    removeTabFromFolder,
    reorderFolders,
    getPinnedTabsGrouped,
    spaces,
    hasOpenTransaction
  } = useAppStore();

  const activeSpace = spaces.find(s => s.id === activeSpaceId);
  const spaceColor = activeSpace?.color || 'var(--accent-color)';

  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; tab: Tab } | null>(null);
  const [folderContextMenu, setFolderContextMenu] = useState<{ x: number; y: number; folder: TabFolder } | null>(null);
  const [tabToRename, setTabToRename] = useState<string | null>(null);
  const [folderToRename, setFolderToRename] = useState<string | null>(null);
  const [createFolderDialogOpen, setCreateFolderDialogOpen] = useState(false);
  const [createFolderForTab, setCreateFolderForTab] = useState<string | null>(null);
  const [dragTabsToCreateFolder, setDragTabsToCreateFolder] = useState<[string, string] | null>(null);

  // Drag state for dnd-kit (pinned tabs section)
  const [activeId, setActiveId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  const unpinnedTabs = tabs.filter(t => !t.is_pinned);
  const { ungrouped: ungroupedPinnedTabs, folders: foldersWithTabs } = getPinnedTabsGrouped();

  // Configure dnd-kit sensors
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8, // 8px movement required before drag starts
      },
    })
  );

  // Close context menus on click outside
  useEffect(() => {
    const handleClick = () => {
      setContextMenu(null);
      setFolderContextMenu(null);
    };
    document.addEventListener('click', handleClick);
    return () => document.removeEventListener('click', handleClick);
  }, []);

  const handleCreateTab = async () => {
    if (onNewTabClick) {
      onNewTabClick();
    }
  };

  // Stable handlers for TabItem
  const handleRename = useCallback(async (id: string, newTitle: string) => {
    await updateTab(id, { title: newTitle });
  }, [updateTab]);

  const handleDeleteTab = useCallback(async (id: string) => {
    await deleteTab(id);
  }, [deleteTab]);

  const handleTogglePin = useCallback(async (id: string) => {
    await toggleTabPinned(id);
  }, [toggleTabPinned]);

  const handleExportTab = useCallback(async (tab: Tab) => {
    const { addToast } = useAppStore.getState();

    try {
      const defaultFilename = api.sanitizeFilename(tab.title);
      const filePath = await api.saveSqlFileDialog(defaultFilename);
      if (!filePath) return; // User cancelled

      // Strip sticky notes from exported content
      let contentToExport = undefined;
      if (tab.content?.includes('-- @note: ')) {
        const { removeNotes } = await import('../utils/noteManager');
        contentToExport = removeNotes(tab.content);
      }

      await api.exportTabAsSql(tab.id, filePath, contentToExport);
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
  }, []);

  const handleTabContextMenu = useCallback((e: React.MouseEvent, tab: Tab) => {
    setContextMenu({ x: e.clientX, y: e.clientY, tab });
  }, []);

  const handleSetActive = useCallback((id: string) => {
    setActiveTab(id);
  }, [setActiveTab]);

  // Folder handlers
  const handleFolderContextMenu = useCallback((e: React.MouseEvent, folder: TabFolder) => {
    setFolderContextMenu({ x: e.clientX, y: e.clientY, folder });
  }, []);

  const handleRenameFolder = useCallback(async (folderId: string, newName: string) => {
    await updateFolder(folderId, { name: newName });
  }, [updateFolder]);

  const handleDeleteFolder = useCallback(async (folderId: string) => {
    await deleteFolder(folderId);
  }, [deleteFolder]);

  const handleToggleFolderExpanded = useCallback((folderId: string) => {
    toggleFolderExpanded(folderId);
  }, [toggleFolderExpanded]);

  const handleCreateFolderForTab = useCallback((tabId: string) => {
    setCreateFolderForTab(tabId);
    setCreateFolderDialogOpen(true);
  }, []);

  const handleRemoveTabFromFolder = useCallback(async (tabId: string) => {
    await removeTabFromFolder(tabId);
  }, [removeTabFromFolder]);

  const handleFolderRenameStarted = useCallback(() => {
    setFolderToRename(null);
  }, []);

  const handleTabRenameStarted = useCallback(() => {
    setTabToRename(null);
  }, []);

  // dnd-kit drag handlers for pinned tabs and folders
  const handleDragStart = useCallback((event: DragStartEvent) => {
    setActiveId(event.active.id as string);
  }, []);

  const handleDragOver = useCallback((event: DragOverEvent) => {
    setOverId(event.over?.id as string | null);
  }, []);

  const handleDragEnd = useCallback((event: DragEndEvent) => {
    const { active, over } = event;
    setActiveId(null);
    setOverId(null);

    if (!over || active.id === over.id) return;

    const activeIdStr = active.id as string;
    const overIdStr = over.id as string;

    // Parse drag types
    const isActiveTab = activeIdStr.startsWith('pinned-tab-') || activeIdStr.startsWith('folder-tab-');
    const isActiveFolder = activeIdStr.startsWith('folder-');
    const isOverTab = overIdStr.startsWith('pinned-tab-') || overIdStr.startsWith('folder-tab-');
    const isOverFolder = overIdStr.startsWith('folder-header-');
    const isOverUngrouped = overIdStr === 'drop-ungrouped';

    // Extract IDs
    const getTabId = (id: string) => id.replace(/^(pinned-tab-|folder-tab-)/, '');
    const getFolderId = (id: string) => id.replace(/^(folder-header-|folder-)/, ''); // Must check folder-header- first!

    // SCENARIO 1: Drag pinned tab onto another pinned tab → Create folder
    if (isActiveTab && isOverTab && activeIdStr.startsWith('pinned-tab-') && overIdStr.startsWith('pinned-tab-')) {
      const tab1Id = getTabId(activeIdStr);
      const tab2Id = getTabId(overIdStr);
      setDragTabsToCreateFolder([tab1Id, tab2Id]);
      setCreateFolderDialogOpen(true);
      return;
    }

    // SCENARIO 2: Drag tab onto folder → Add to folder
    if (isActiveTab && isOverFolder && activeSpaceId) {
      const tabId = getTabId(activeIdStr);
      const folderId = getFolderId(overIdStr);
      addTabToFolder(tabId, folderId).catch(err => {
        console.error('Failed to add tab to folder:', err);
      });
      return;
    }

    // SCENARIO 3: Drag folder tab to ungrouped → Remove from folder
    if (isActiveTab && activeIdStr.startsWith('folder-tab-') && isOverUngrouped) {
      const tabId = getTabId(activeIdStr);
      removeTabFromFolder(tabId).catch(err => {
        console.error('Failed to remove tab from folder:', err);
      });
      return;
    }

    // SCENARIO 4: Drag folder to reorder (folder onto folder)
    if (isActiveFolder && isOverFolder && activeSpaceId) {
      const activeFolderId = getFolderId(activeIdStr);
      const overFolderId = getFolderId(overIdStr);

      // Reorder folders
      const folderIds = foldersWithTabs.map(f => f.folder.id);
      const activeIndex = folderIds.indexOf(activeFolderId);
      const overIndex = folderIds.indexOf(overFolderId);

      if (activeIndex !== -1 && overIndex !== -1) {
        const newOrder = [...folderIds];
        newOrder.splice(activeIndex, 1);
        newOrder.splice(overIndex, 0, activeFolderId);
        reorderFolders(newOrder).catch(err => {
          console.error('Failed to reorder folders:', err);
        });
      }
      return;
    }
  }, [activeSpaceId, foldersWithTabs, addTabToFolder, removeTabFromFolder, reorderFolders]);

  // Handle folder creation from drag-and-drop
  const handleCreateFolderFromDrag = useCallback(async (name: string) => {
    if (dragTabsToCreateFolder && activeSpaceId) {
      await createFolderFromTabs(activeSpaceId, name, dragTabsToCreateFolder);
      setDragTabsToCreateFolder(null);
    } else if (createFolderForTab && activeSpaceId) {
      await createFolderFromTabs(activeSpaceId, name, [createFolderForTab]);
      setCreateFolderForTab(null);
    } else if (activeSpaceId) {
      await createFolder(activeSpaceId, name);
    }
  }, [dragTabsToCreateFolder, createFolderForTab, activeSpaceId, createFolderFromTabs, createFolder]);

  // Drag-and-drop handler for unpinned tabs
  const onDragEnd = useCallback(
    (result: DropResult) => {
      if (!result.destination) return;
      const from = result.source.index;
      const to = result.destination.index;
      if (from === to) return;
      const newOrder = Array.from(unpinnedTabs);
      const [removed] = newOrder.splice(from, 1);
      newOrder.splice(to, 0, removed);
      reorderTabs(newOrder.map(t => t.id)).catch(err => {
        console.error('Failed to reorder tabs:', err);
      });
    },
    [reorderTabs, unpinnedTabs]
  );

  if (!activeSpaceId) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-6 text-center">
        <div className="w-12 h-12 rounded-2xl bg-white/5 flex items-center justify-center mb-3">
          <svg className="w-6 h-6 text-[--text-muted]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
          </svg>
        </div>
        <p className="text-[--text-muted] text-sm">Create a space to get started</p>
      </div>
    );
  }

  if (tabsLoading) {
    return (
      <div className="flex-1 flex flex-col min-h-0 py-2">
        {/* Skeleton tabs */}
        <div className="space-y-1 px-1">
          {[1, 2, 3].map((i) => (
            <div key={i} className="flex items-center gap-2 px-2 py-1.5 mx-1 rounded-lg">
              <div className="w-4 h-4 rounded bg-white/10 animate-pulse" />
              <div className="h-3.5 rounded bg-white/10 animate-pulse" style={{ width: `${60 + i * 15}%` }} />
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col min-h-0">

      {/* Pinned section with folders (dnd-kit drag-and-drop) */}
      {(ungroupedPinnedTabs.length > 0 || foldersWithTabs.length > 0) && (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCorners}
          onDragStart={handleDragStart}
          onDragOver={handleDragOver}
          onDragEnd={handleDragEnd}
        >
          <div className="py-2">
            <div className="px-4 mb-1">
              <span className="text-[10px] font-semibold text-[--text-muted] uppercase tracking-wider">
                Pinned
              </span>
            </div>

            {/* Ungrouped pinned tabs */}
            {ungroupedPinnedTabs.map(tab => (
              <DraggableTabItem
                key={tab.id}
                tab={tab}
                isOver={overId === `pinned-tab-${tab.id}`}
                isPinned={true}
                isActive={tab.id === activeTabId}
                onRename={handleRename}
                onDelete={handleDeleteTab}
                shouldStartRename={tabToRename === tab.id}
                onContextMenu={handleTabContextMenu}
                onRenameStarted={handleTabRenameStarted}
                onSetActive={handleSetActive}
                spaceColor={spaceColor}
                hasOpenTransaction={hasOpenTransaction[tab.id]}
              />
            ))}

            {/* Drop zone for removing tabs from folders (only show when dragging a folder tab) */}
            {activeId && activeId.startsWith('folder-tab-') && (
              <UngroupedDropZone isOver={overId === 'drop-ungrouped'}>
                <div className="px-3 py-2 text-xs text-[var(--text-muted)] text-center">
                  Drop here to remove from folder
                </div>
              </UngroupedDropZone>
            )}

            {/* Folders with tabs */}
            {foldersWithTabs.map(({ folder, tabs }) => (
              <DraggableFolder
                key={folder.id}
                folder={folder}
                tabs={tabs}
                isOver={overId === `folder-header-${folder.id}`}
                activeTabId={activeTabId}
                onToggleExpanded={handleToggleFolderExpanded}
                onRenameFolder={handleRenameFolder}
                onRenameTab={handleRename}
                onDeleteTab={handleDeleteTab}
                onSetActiveTab={handleSetActive}
                spaceColor={spaceColor}
                onFolderContextMenu={handleFolderContextMenu}
                onTabContextMenu={handleTabContextMenu}
                shouldStartRenamingFolder={folderToRename === folder.id}
                onFolderRenameStarted={handleFolderRenameStarted}
                tabToRename={tabToRename}
                onTabRenameStarted={handleTabRenameStarted}
              />
            ))}
          </div>

          {/* Drag overlay for visual feedback */}
          <DragOverlay
            dropAnimation={{
              duration: 200,
              easing: 'cubic-bezier(0.18, 0.67, 0.6, 1.22)',
            }}
            style={{
              cursor: 'grabbing',
            }}
          >
            {activeId && activeId.startsWith('pinned-tab-') && (() => {
              const tabId = activeId.replace('pinned-tab-', '');
              const tab = ungroupedPinnedTabs.find(t => t.id === tabId);
              return tab ? (
                <div style={{
                  opacity: 0.8,
                  cursor: 'grabbing',
                  transform: 'translate(-50%, -50%)',
                  pointerEvents: 'none',
                }}>
                  <TabItem
                    tab={tab}
                    isPinned={true}
                    isActive={false}
                    onRename={() => { }}
                    onDelete={() => { }}
                    onSetActive={() => { }}
                    spaceColor={spaceColor}
                  />
                </div>
              ) : null;
            })()}
            {activeId && activeId.startsWith('folder-') && (() => {
              const folderId = activeId.replace('folder-', '');
              const folderData = foldersWithTabs.find(f => f.folder.id === folderId);
              return folderData ? (
                <div style={{
                  opacity: 0.8,
                  cursor: 'grabbing',
                  transform: 'translate(-50%, -50%)',
                  pointerEvents: 'none',
                }}>
                  <FolderItem
                    folder={folderData.folder}
                    tabs={folderData.tabs}
                    activeTabId={null}
                    onToggleExpanded={() => { }}
                    onRenameFolder={() => { }}
                    onRenameTab={() => { }}
                    onDeleteTab={() => { }}
                    onSetActiveTab={() => { }}
                    spaceColor={spaceColor}
                  />
                </div>
              ) : null;
            })()}
          </DragOverlay>
        </DndContext>
      )}

      {/* Divider between sections */}
      {(ungroupedPinnedTabs.length > 0 || foldersWithTabs.length > 0) && unpinnedTabs.length > 0 && (
        <div className="mx-4 border-t border-white/5" />
      )}

      {/* Tabs section */}
      <div
        className="flex-1 overflow-y-auto"
        onMouseDown={(e) => {
          // Middle click (button 1) on blank space creates a new tab
          if (e.button === 1 && e.target === e.currentTarget) {
            e.preventDefault();
            handleCreateTab();
          }
        }}
      >
        {/* Browser-style New Tab button - disabled when no spaces */}


        {/* Unpinned tabs with drag-and-drop */}
        <DragDropContext onDragEnd={onDragEnd}>
          <Droppable droppableId="tabs-droppable">
            {(provided) => (
              <div ref={provided.innerRef} {...provided.droppableProps}>
                {unpinnedTabs.map((tab, index) => (
                  <Draggable key={tab.id} draggableId={tab.id} index={index}>
                    {(provided, snapshot) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.draggableProps}
                        {...provided.dragHandleProps}
                        style={{
                          ...provided.draggableProps.style,
                          transform: snapshot.isDragging ? 'translateY(-50%)' : 'none',
                          opacity: snapshot.isDragging ? 0.8 : 1,
                        }}
                      >
                        <TabItem
                          tab={tab}
                          isPinned={false}
                          isActive={tab.id === activeTabId}
                          onRename={handleRename}
                          onDelete={handleDeleteTab}
                          shouldStartRename={tabToRename === tab.id}
                          onContextMenu={handleTabContextMenu}
                          onRenameStarted={handleTabRenameStarted}
                          onSetActive={handleSetActive}
                          spaceColor={spaceColor}
                          hasOpenTransaction={hasOpenTransaction[tab.id]}
                        />
                      </div>
                    )}
                  </Draggable>
                ))}
                {provided.placeholder}
              </div>
            )}
          </Droppable>
        </DragDropContext>

        {/* Empty state message */}
        {tabs.length === 0 && (
          <div className="flex flex-col items-center justify-center p-6 text-center">
            <p className="text-[--text-muted] text-sm">No tabs yet</p>
            <p className="text-[--text-muted] text-xs mt-1">Click "New Tab" above to create one</p>
          </div>
        )}
      </div>

      {/* Tab context menu */}
      {
        contextMenu && (
          <ContextMenu
            position={{ x: contextMenu.x, y: contextMenu.y }}
            onClose={() => setContextMenu(null)}
            items={[
              {
                id: 'pin',
                label: contextMenu.tab.is_pinned ? 'Unpin Tab' : 'Pin Tab',
                icon: contextMenu.tab.is_pinned ? (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                ) : (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
                  </svg>
                ),
                shortcut: 'Ctrl+P',
                action: () => handleTogglePin(contextMenu.tab.id),
              },
              // Show folder options for pinned tabs
              ...(contextMenu.tab.is_pinned && contextMenu.tab.folder_id ? [
                {
                  id: 'remove-from-folder',
                  label: 'Remove from Folder',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  ),
                  action: () => handleRemoveTabFromFolder(contextMenu.tab.id),
                },
              ] : contextMenu.tab.is_pinned ? [
                {
                  id: 'add-to-new-folder',
                  label: 'Add to New Folder',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 13h6m-3-3v6m-9 1V7a2 2 0 012-2h6l2 2h6a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
                    </svg>
                  ),
                  action: () => handleCreateFolderForTab(contextMenu.tab.id),
                },
              ] : []),
              {
                id: 'rename',
                label: 'Rename',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                  </svg>
                ),
                shortcut: 'F2',
                action: () => {
                  const tabId = contextMenu.tab.id;
                  setTimeout(() => setTabToRename(tabId), 0);
                },
              },
              {
                id: 'export',
                label: 'Export as SQL File...',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                  </svg>
                ),
                shortcut: 'Ctrl+Shift+E',
                action: () => handleExportTab(contextMenu.tab),
              },
              {
                id: 'delete',
                label: 'Delete',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                ),
                shortcut: 'Ctrl+W',
                separator: true,
                danger: true,
                action: () => handleDeleteTab(contextMenu.tab.id),
              },
            ]}
          />
        )
      }

      {/* Folder context menu */}
      {
        folderContextMenu && (
          <ContextMenu
            position={{ x: folderContextMenu.x, y: folderContextMenu.y }}
            onClose={() => setFolderContextMenu(null)}
            items={[
              {
                id: 'rename-folder',
                label: 'Rename Folder',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                  </svg>
                ),
                shortcut: 'F2',
                action: () => {
                  const folderId = folderContextMenu.folder.id;
                  setTimeout(() => setFolderToRename(folderId), 0);
                },
              },
              {
                id: 'toggle-folder',
                label: folderContextMenu.folder.is_expanded ? 'Collapse Folder' : 'Expand Folder',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    {folderContextMenu.folder.is_expanded ? (
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                    ) : (
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                    )}
                  </svg>
                ),
                action: () => handleToggleFolderExpanded(folderContextMenu.folder.id),
              },
              {
                id: 'delete-folder',
                label: 'Delete Folder',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                ),
                separator: true,
                danger: true,
                action: () => handleDeleteFolder(folderContextMenu.folder.id),
              },
            ]}
          />
        )
      }

      {/* Create Folder Dialog */}
      <CreateFolderDialog
        isOpen={createFolderDialogOpen}
        onClose={() => {
          setCreateFolderDialogOpen(false);
          setCreateFolderForTab(null);
          setDragTabsToCreateFolder(null);
        }}
        onCreateFolder={handleCreateFolderFromDrag}
      />
    </div >
  );
}
