import { useState, useRef, useEffect, useCallback } from 'react';
import { Minimize2, Maximize2, Trash2, X, Pin, PinOff } from 'lucide-react';
import type { StickyNote } from '../api/notes';
import { getReadableTextColor } from '../utils/color';

// Theme-aware semantic color palette.
// Keys: yellow, blue, green, orange, pink, purple.
// Each has light and dark variants for proper contrast in both themes.
export const NOTE_COLORS: Record<string, { light: string; dark: string; label: string }> = {
  yellow: { light: '#fef3c7', dark: '#7c5e10', label: 'Yellow' },
  blue:   { light: '#dbeafe', dark: '#1e3a5f', label: 'Blue' },
  green:  { light: '#d1fae5', dark: '#14532d', label: 'Green' },
  orange: { light: '#fed7aa', dark: '#7c2d12', label: 'Orange' },
  pink:   { light: '#fce7f3', dark: '#6b213f', label: 'Pink' },
  purple: { light: '#ede9fe', dark: '#4c1d95', label: 'Purple' },
};

export const NOTE_COLOR_KEYS = Object.keys(NOTE_COLORS);

export function getNoteColorBg(colorKey: string, theme: 'dark' | 'light'): string {
  const entry = NOTE_COLORS[colorKey];
  if (!entry) return NOTE_COLORS.yellow[theme];
  return entry[theme];
}

export interface NotePopoverProps {
  note: StickyNote;
  position: { top: number; left: number };
  /** Stack z-index — higher means more recently accessed (on top). */
  zIndex: number;
  theme: 'dark' | 'light';
  onChange: (noteId: string, updates: Partial<StickyNote>) => void;
  onDelete: (noteId: string) => void;
  /** Close this popover. Receives the note id so the parent can target it. */
  onClose: (noteId: string) => void;
  /** Bring this popover to the front of the stack. */
  onFocus: (noteId: string) => void;
}

export const NotePopover: React.FC<NotePopoverProps> = ({
  note,
  position,
  zIndex,
  theme,
  onChange,
  onDelete,
  onClose,
  onFocus,
}) => {
  const [content, setContent] = useState(note.content);
  const [color, setColor] = useState(note.color);
  const [isMinimized, setIsMinimized] = useState(note.minimized);
  const [pinned, setPinned] = useState(note.pinned ?? false);
  // Persisted resizable width. Kept in state (not just the DOM) so it survives
  // re-renders — otherwise typing would snap the popover back to the default.
  const [width, setWidth] = useState<number>(note.width ?? 380);
  const popoverRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Track user resize (CSS `resize: horizontal`) and mirror it into state.
  useEffect(() => {
    const el = popoverRef.current;
    if (!el || isMinimized || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      // Use offsetWidth (layout border-box), NOT getBoundingClientRect —
      // the latter includes the entry `scale()` transform, which would make
      // the saved width shrink a little on every open/close cycle.
      const w = el.offsetWidth;
      if (w > 0) setWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [isMinimized]);

  // Focus the textarea on mount
  useEffect(() => {
    if (textareaRef.current && !isMinimized) {
      textareaRef.current.focus();
    }
  }, [isMinimized]);

  // Auto-resize textarea
  useEffect(() => {
    if (textareaRef.current && !isMinimized) {
      textareaRef.current.style.height = 'auto';
      textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
    }
  }, [content, isMinimized]);

  // Close on Escape (disabled while pinned)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (pinned) return;
        if (content !== note.content || color !== note.color || isMinimized !== note.minimized || width !== note.width || pinned !== (note.pinned ?? false)) {
          onChange(note.id, { content, color, minimized: isMinimized, width, pinned });
        }
        onClose(note.id);
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [content, color, isMinimized, width, pinned, note.id, note.content, note.color, note.minimized, note.width, note.pinned, onChange, onClose]);

  const handleColorChange = useCallback(
    (newColor: string) => {
      setColor(newColor);
      onChange(note.id, { color: newColor, minimized: isMinimized, content, width });
    },
    [content, isMinimized, width, note.id, onChange],
  );

  const handleToggleMinimize = useCallback(() => {
    const newState = !isMinimized;
    setIsMinimized(newState);
    onChange(note.id, { minimized: newState, content, color, width });
  }, [content, color, isMinimized, width, note.id, onChange]);

  const handleTogglePin = useCallback(() => {
    const newState = !pinned;
    setPinned(newState);
    onChange(note.id, { pinned: newState, content, color, minimized: isMinimized, width });
  }, [content, color, isMinimized, width, pinned, note.id, onChange]);

  const handleDelete = useCallback(() => {
    onDelete(note.id);
    onClose(note.id);
  }, [note.id, onDelete, onClose]);

  const handleContentChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setContent(e.target.value);
  }, []);

  const handleContentBlur = useCallback(() => {
    onChange(note.id, { content, color, minimized: isMinimized, width });
  }, [content, color, isMinimized, width, note.id, onChange]);

  const bgColor = getNoteColorBg(color, theme);
  const fgColor = getReadableTextColor(bgColor);

  const popoverStyle: React.CSSProperties = {
    position: 'fixed',
    top: position.top,
    left: position.left,
    zIndex: zIndex,
    backgroundColor: bgColor,
    color: fgColor,
    borderRadius: '8px',
    boxShadow: '0 8px 24px rgba(0,0,0,0.25), 0 2px 8px rgba(0,0,0,0.15)',
    border: `1px solid ${theme === 'dark' ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)'}`,
    fontFamily: '-apple-system, BlinkMacSystemFont, \'SF Pro Display\', \'Segoe UI\', Roboto, sans-serif',
    animation: 'larik-note-popover-in 150ms cubic-bezier(0.4, 0, 0.2, 1)',
    // Persisted width; drag the right edge to resize (vertical height
    // still auto-grows with content).
    width: isMinimized ? undefined : width,
    minWidth: isMinimized ? '180px' : '260px',
    maxWidth: isMinimized ? '320px' : '760px',
    resize: isMinimized ? undefined : 'horizontal',
    overflow: isMinimized ? undefined : 'hidden',
  };

  const handleCloseWithFlush = useCallback(() => {
    if (pinned) return;
    if (content !== note.content || color !== note.color || isMinimized !== note.minimized || width !== note.width || pinned !== (note.pinned ?? false)) {
      onChange(note.id, { content, color, minimized: isMinimized, width, pinned });
    }
    onClose(note.id);
  }, [content, color, isMinimized, width, pinned, note.id, note.content, note.color, note.minimized, note.width, note.pinned, onChange, onClose]);

  if (isMinimized) {
    return (
      <>
        {/* Transparent backdrop — catches clicks outside the popover.
            Omitted while pinned so outside clicks pass through to the editor. */}
        {!pinned && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 999,
            background: 'transparent',
          }}
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            handleCloseWithFlush();
          }}
        />
        )}
        <div
          ref={popoverRef}
          style={{ ...popoverStyle, position: 'fixed' }}
          className="larik-note-popover select-none cursor-pointer"
          onClick={(e) => {
            e.stopPropagation();
            handleToggleMinimize();
          }}
          onMouseDown={(e) => {
            e.stopPropagation();
            onFocus(note.id);
          }}
        >
          <div className="flex items-center gap-2 px-3 py-1.5">
            <span className="truncate text-xs font-medium flex-1" style={{ color: fgColor }}>
              {content || '(Empty note)'}
            </span>
            <Maximize2 size={12} style={{ color: fgColor, opacity: 0.6 }} />
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      {/* Transparent backdrop — catches clicks outside the popover.
          Omitted while pinned so outside clicks pass through to the editor. */}
      {!pinned && (
      <div
        style={{
          position: 'fixed',
          inset: 0,
          zIndex: 999,
          background: 'transparent',
        }}
        onMouseDown={(e) => {
          e.stopPropagation();
          e.preventDefault();
          handleCloseWithFlush();
        }}
      />
      )}
      {/* The actual popover card */}
      <div
        ref={popoverRef}
        style={{ ...popoverStyle, position: 'fixed' }}
        className="larik-note-popover"
        onMouseDown={(e) => {
          e.stopPropagation();
          onFocus(note.id);
        }}
      >
      {/* Header / Toolbar */}
      <div
        className="flex items-center justify-between px-2 py-1 border-b"
        style={{
          borderColor: theme === 'dark' ? 'rgba(255,255,255,0.1)' : 'rgba(0,0,0,0.08)',
          backgroundColor: theme === 'dark' ? 'rgba(0,0,0,0.15)' : 'rgba(0,0,0,0.05)',
        }}
      >
        {/* Color picker */}
        <div className="flex items-center gap-1">
          {NOTE_COLOR_KEYS.map((key) => {
            const c = getNoteColorBg(key, theme);
            return (
              <button
                key={key}
                className="w-3 h-3 rounded-full border transition-transform hover:scale-125"
                style={{
                  backgroundColor: c,
                  borderColor: 'rgba(0,0,0,0.1)',
                  outline: color === key ? `1.5px solid ${fgColor}` : 'none',
                  outlineOffset: '1px',
                }}
                onClick={(e) => {
                  e.stopPropagation();
                  handleColorChange(key);
                }}
                onMouseDown={(e) => e.stopPropagation()}
                title={NOTE_COLORS[key].label}
              />
            );
          })}
        </div>

        {/* Action buttons */}
        <div className="flex items-center gap-0.5">
          <button
            onClick={(e) => {
              e.stopPropagation();
              handleTogglePin();
            }}
            onMouseDown={(e) => e.stopPropagation()}
            className="p-1 rounded transition-colors hover:bg-black/10"
            style={{ color: fgColor, opacity: pinned ? 1 : 0.7 }}
            title={pinned ? 'Unpin (outside clicks won\'t close the note)' : 'Pin (keep note open on outside click)'}
          >
            {pinned ? <PinOff size={12} /> : <Pin size={12} />}
          </button>
          <button
            onClick={(e) => {
              e.stopPropagation();
              handleToggleMinimize();
            }}
            onMouseDown={(e) => e.stopPropagation()}
            className="p-1 rounded transition-colors"
            style={{ color: fgColor, opacity: 0.7 }}
            title="Minimize"
          >
            <Minimize2 size={12} />
          </button>
          <button
            onClick={(e) => {
              e.stopPropagation();
              handleDelete();
            }}
            onMouseDown={(e) => e.stopPropagation()}
            className="p-1 rounded transition-colors hover:text-red-500"
            style={{ color: fgColor, opacity: 0.7 }}
            title="Delete note"
          >
            <Trash2 size={12} />
          </button>
          <button
onClick={(e) => {
            e.stopPropagation();
            if (content !== note.content || color !== note.color || isMinimized !== note.minimized || width !== note.width || pinned !== (note.pinned ?? false)) {
              onChange(note.id, { content, color, minimized: isMinimized, width, pinned });
            }
            onClose(note.id);
          }}
            onMouseDown={(e) => e.stopPropagation()}
            className="p-1 rounded transition-colors"
            style={{ color: fgColor, opacity: 0.7 }}
            title="Close"
          >
            <X size={12} />
          </button>
        </div>
      </div>

      {/* Content textarea */}
      <div>
        <textarea
          ref={textareaRef}
          value={content}
          onChange={handleContentChange}
          onBlur={handleContentBlur}
          placeholder="Write a note..."
          className="w-full bg-transparent resize-none focus:outline-none text-sm leading-relaxed"
          style={{
            color: fgColor,
            minHeight: '60px',
            fontFamily: 'inherit',
            // Padding lives on the textarea itself so the text always sits
            // inset from the note edges (the global `textarea` rule in
            // index.css strips padding, so we set it explicitly here).
            padding: '12px 16px',
            margin: 0,
            border: 'none',
            outline: 'none',
            background: 'transparent',
            boxSizing: 'border-box',
            display: 'block',
          }}
          onMouseDown={(e) => e.stopPropagation()}
        />
      </div>
      </div>
    </>
  );
};
