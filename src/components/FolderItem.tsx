// Folder Item Component - Arc Browser-style folder with expand/collapse
import { useState, useRef, useEffect, memo, useCallback } from 'react';
import { useDraggable } from '@dnd-kit/core';
import type { Tab, TabFolder } from '../types';

// Tab item for tabs inside folders (indented)
const FolderTabItem = memo(({
  tab,
  isActive,
  onSetActive,
  onRename,
  onDelete,
  shouldStartRename,
  onContextMenu,
  onRenameStarted,
}: {
  tab: Tab;
  isActive: boolean;
  onSetActive: (id: string) => void;
  onRename: (id: string, newTitle: string) => void;
  onDelete: (id: string) => void;
  shouldStartRename?: boolean;
  onContextMenu?: (e: React.MouseEvent, tab: Tab) => void;
  onRenameStarted?: () => void;
}) => {
  const [isRenaming, setIsRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isRenaming && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [isRenaming]);

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

  // Make folder tabs draggable
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({
    id: `folder-tab-${tab.id}`,
    data: { type: 'folder-tab', tab },
  });

  const style: React.CSSProperties = {
    transform: transform ? `translate3d(${transform.x}px, ${transform.y}px, 0)` : undefined,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <div
      ref={setNodeRef}
      aria-label={`Tab: ${tab.title}${tab.database ? ` (${tab.database})` : ''}`}
      aria-current={isActive ? 'page' : undefined}
      style={style}
      {...attributes}
      {...listeners}
      className={`
        group flex items-center gap-2 px-2 py-1.5 ml-6 mr-1 my-0.5 rounded-lg cursor-pointer
        transition-all duration-150 ease-out focus:outline-none focus:ring-2 focus:ring-[var(--accent-color)] focus:ring-offset-1
        ${isActive
          ? 'bg-[var(--bg-active)] shadow-sm'
          : 'hover:bg-[var(--bg-hover)]'
        }
      `}
      onClick={() => onSetActive(tab.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          onSetActive(tab.id);
        } else if (e.key === 'F2') {
          e.preventDefault();
          handleStartRename();
        } else if (e.key === 'Delete') {
          e.preventDefault();
          onDelete(tab.id);
        }
      }}
      onDoubleClick={handleStartRename}
      onMouseDown={(e) => {
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
      <div className={`shrink-0 ${isActive ? 'text-[--accent-color]' : 'text-[--text-muted]'}`}>
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
        </svg>
      </div>

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
          {tab.database && (
            <span className="truncate text-[10px] text-[--text-muted] leading-tight">
              {tab.database}
            </span>
          )}
        </div>
      )}
    </div>
  );
});

interface FolderItemProps {
  folder: TabFolder;
  tabs: Tab[];
  activeTabId: string | null;
  onToggleExpanded: (folderId: string) => void;
  onRenameFolder: (folderId: string, newName: string) => void;
  onRenameTab: (tabId: string, newTitle: string) => void;
  onDeleteTab: (tabId: string) => void;
  onSetActiveTab: (tabId: string) => void;
  onFolderContextMenu?: (e: React.MouseEvent, folder: TabFolder) => void;
  onTabContextMenu?: (e: React.MouseEvent, tab: Tab) => void;
  shouldStartRenamingFolder?: boolean;
  onFolderRenameStarted?: () => void;
  tabToRename?: string | null;
  onTabRenameStarted?: () => void;
  dragHandleRef?: (element: HTMLElement | null) => void;
  dragHandleAttributes?: Record<string, any>;
  dragHandleListeners?: Record<string, any>;
}

export const FolderItem = memo(({
  folder,
  tabs,
  activeTabId,
  onToggleExpanded,
  onRenameFolder,
  onRenameTab,
  onDeleteTab,
  onSetActiveTab,
  onFolderContextMenu,
  onTabContextMenu,
  shouldStartRenamingFolder,
  onFolderRenameStarted,
  tabToRename,
  onTabRenameStarted,
  dragHandleRef,
  dragHandleAttributes,
  dragHandleListeners,
}: FolderItemProps) => {
  const [isRenamingFolder, setIsRenamingFolder] = useState(false);
  const [renameFolderValue, setRenameFolderValue] = useState('');
  const folderInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isRenamingFolder && folderInputRef.current) {
      folderInputRef.current.focus();
      folderInputRef.current.select();
    }
  }, [isRenamingFolder]);

  useEffect(() => {
    if (shouldStartRenamingFolder && !isRenamingFolder) {
      setIsRenamingFolder(true);
      setRenameFolderValue(folder.name);
      onFolderRenameStarted?.();
    }
  }, [shouldStartRenamingFolder, isRenamingFolder, folder.name, onFolderRenameStarted]);

  const handleStartRenameFolder = () => {
    setIsRenamingFolder(true);
    setRenameFolderValue(folder.name);
  };

  const handleFinishRenameFolder = () => {
    if (renameFolderValue.trim() && renameFolderValue !== folder.name) {
      onRenameFolder(folder.id, renameFolderValue.trim());
    }
    setIsRenamingFolder(false);
    setRenameFolderValue('');
  };

  const handleCancelRenameFolder = () => {
    setIsRenamingFolder(false);
    setRenameFolderValue('');
  };

  const handleToggleExpanded = useCallback((e: React.MouseEvent) => {
    // Don't toggle if clicking on the rename input
    if (isRenamingFolder) return;
    e.stopPropagation();
    onToggleExpanded(folder.id);
  }, [folder.id, onToggleExpanded, isRenamingFolder]);

  return (
    <div className="my-0.5">
      {/* Folder header */}
      <div
        role="button"
        tabIndex={0}
        aria-label={`Folder: ${folder.name}, ${tabs.length} tab${tabs.length !== 1 ? 's' : ''}`}
        aria-expanded={folder.is_expanded}
        className="group flex items-center gap-2 px-2 py-1.5 mx-1 rounded-lg cursor-pointer hover:bg-[var(--bg-hover)] transition-all duration-150 ease-out focus:outline-none focus:ring-2 focus:ring-[var(--accent-color)] focus:ring-offset-1"
        onClick={handleToggleExpanded}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            handleToggleExpanded(e as any);
          } else if (e.key === 'F2') {
            e.preventDefault();
            handleStartRenameFolder();
          }
        }}
        onDoubleClick={(e) => {
          e.stopPropagation();
          handleStartRenameFolder();
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          onFolderContextMenu?.(e, folder);
        }}
      >
        {/* Chevron - also serves as drag handle */}
        <div
          ref={dragHandleRef}
          className="shrink-0 text-[--text-muted] cursor-grab active:cursor-grabbing"
          {...dragHandleAttributes}
          {...dragHandleListeners}
        >
          <svg
            className={`w-4 h-4 transition-transform ${folder.is_expanded ? 'rotate-90' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        </div>

        {/* Folder icon */}
        <div className="shrink-0 text-[--text-muted]">
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.5}
              d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
            />
          </svg>
        </div>

        {/* Folder name */}
        {isRenamingFolder ? (
          <input
            ref={folderInputRef}
            type="text"
            value={renameFolderValue}
            onChange={(e) => setRenameFolderValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleFinishRenameFolder();
              if (e.key === 'Escape') handleCancelRenameFolder();
            }}
            onBlur={handleFinishRenameFolder}
            onClick={(e) => e.stopPropagation()}
            className="flex-1 min-w-0 text-sm font-medium bg-white/10 px-2 py-0.5 rounded outline-none text-[var(--text-primary)]"
          />
        ) : (
          <span className="flex-1 min-w-0 truncate text-sm font-medium text-[var(--text-primary)]">
            {folder.name}
          </span>
        )}

        {/* Tab count badge */}
        <span className="shrink-0 text-[10px] text-[--text-muted] bg-white/5 px-1.5 py-0.5 rounded">
          {tabs.length}
        </span>
      </div>

      {/* Child tabs (when expanded) with smooth animation */}
      <div
        className="overflow-hidden transition-all duration-200 ease-in-out"
        style={{
          maxHeight: folder.is_expanded ? `${tabs.length * 60}px` : '0px',
          opacity: folder.is_expanded ? 1 : 0,
        }}
      >
        <div className="mt-0.5">
          {tabs.map((tab) => (
            <FolderTabItem
              key={tab.id}
              tab={tab}
              isActive={tab.id === activeTabId}
              onSetActive={onSetActiveTab}
              onRename={onRenameTab}
              onDelete={onDeleteTab}
              shouldStartRename={tabToRename === tab.id}
              onContextMenu={onTabContextMenu}
              onRenameStarted={onTabRenameStarted}
            />
          ))}
        </div>
      </div>
    </div>
  );
});
