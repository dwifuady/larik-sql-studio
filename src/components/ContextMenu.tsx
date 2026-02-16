// Reusable context menu component for right-click actions
import { useEffect, useRef, ReactNode } from 'react';
import { createPortal } from 'react-dom';

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: ReactNode;
  shortcut?: string;
  action: () => void | Promise<void>;
  separator?: boolean;
  danger?: boolean;
  disabled?: boolean;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number };
  onClose: () => void;
}

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Escape
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };

    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };

    // Slight delay to prevent immediate close from the triggering click
    setTimeout(() => {
      document.addEventListener('mousedown', handleClickOutside);
      document.addEventListener('keydown', handleEscape);
    }, 100);

    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleEscape);
    };
  }, [onClose]);

  // Adjust position to keep menu in viewport
  useEffect(() => {
    if (menuRef.current) {
      const menu = menuRef.current;
      const rect = menu.getBoundingClientRect();
      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;

      let { x, y } = position;

      // Adjust horizontal position
      if (x + rect.width > viewportWidth) {
        x = viewportWidth - rect.width - 10;
      }

      // Adjust vertical position
      if (y + rect.height > viewportHeight) {
        y = viewportHeight - rect.height - 10;
      }

      menu.style.left = `${Math.max(10, x)}px`;
      menu.style.top = `${Math.max(10, y)}px`;
    }
  }, [position]);

  const handleItemClick = async (item: ContextMenuItem) => {
    if (item.disabled) return;

    try {
      await item.action();
    } catch (error) {
      console.error('Context menu action failed:', error);
    }
    onClose();
  };

  const menuContent = (
    <div
      ref={menuRef}
      className="fixed py-1 min-w-[200px] rounded-lg border border-[var(--border-color)] shadow-xl animate-in fade-in zoom-in-95 duration-100 bg-[var(--bg-secondary)]"
      style={{
        left: position.x,
        top: position.y,
        zIndex: 9999,
        backdropFilter: 'blur(20px)',
      }}
    >
      {items.map((item, index) => (
        <div key={item.id}>
          {item.separator && index > 0 && (
            <div className="my-1 mx-2 border-t border-[var(--border-color)]" />
          )}
          <button
            onClick={() => handleItemClick(item)}
            disabled={item.disabled}
            className={`w-full px-3 py-2 flex items-center gap-3 text-sm transition-colors ${
              item.disabled
                ? 'opacity-50 cursor-not-allowed'
                : item.danger
                ? 'text-red-400 hover:bg-red-500/10'
                : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
            }`}
          >
            {item.icon && (
              <div className="flex-shrink-0 w-4 h-4 flex items-center justify-center">
                {item.icon}
              </div>
            )}
            <span className="flex-1 text-left">{item.label}</span>
            {item.shortcut && (
              <kbd className="flex-shrink-0 text-xs text-[var(--text-muted)] font-mono">
                {item.shortcut}
              </kbd>
            )}
          </button>
        </div>
      ))}
    </div>
  );

  return createPortal(menuContent, document.body);
}
