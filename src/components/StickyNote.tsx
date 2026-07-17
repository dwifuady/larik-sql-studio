import { useState, useEffect, useRef, useCallback } from 'react';
import { Minimize2, Maximize2, Trash2, GripVertical } from 'lucide-react';
import {
    type NoteColorKey,
    NOTE_COLOR_KEYS,
    resolveNoteColor,
    normalizeColorKey,
} from '../utils/noteManager';
import { getReadableTextColor } from '../utils/color';

export interface StickyNoteProps {
    id: string;
    initialContent: string;
    initialColor: string; // semantic key or legacy hex
    initialMinimized?: boolean;
    theme: 'dark' | 'light';
    onChange: (id: string, content: string, color: NoteColorKey, minimized: boolean) => void;
    onDelete: (id: string) => void;
    onResize?: (height: number) => void;
    onDragStart?: (id: string) => void;
    onDragMove?: (deltaLines: number) => void;
    onDragEnd?: () => void;
}

export const StickyNote: React.FC<StickyNoteProps> = ({
    id,
    initialContent,
    initialColor,
    initialMinimized,
    theme,
    onChange,
    onDelete,
    onResize,
    onDragStart,
    onDragMove,
    onDragEnd,
}) => {
    const colorKey = normalizeColorKey(initialColor);
    const [content, setContent] = useState(initialContent);
    const [color, setColor] = useState<NoteColorKey>(colorKey);
    const [isMinimized, setIsMinimized] = useState(!!initialMinimized);
    const [isDragging, setIsDragging] = useState(false);

    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const dragStartYRef = useRef<number>(0);
    const editorLineHeightRef = useRef<number>(20); // approximate Monaco line height
    const suppressBlurRef = useRef<boolean>(false);

    // Sync when note identity changes (tab switch, external edits)
    useEffect(() => {
        setContent(initialContent);
    }, [initialContent]);

    useEffect(() => {
        setColor(normalizeColorKey(initialColor));
    }, [initialColor]);

    useEffect(() => {
        setIsMinimized(!!initialMinimized);
    }, [initialMinimized]);

    // Auto-resize textarea + notify parent of height change
    useEffect(() => {
        if (textareaRef.current && !isMinimized) {
            const ta = textareaRef.current;
            ta.style.height = 'auto';
            ta.style.height = ta.scrollHeight + 'px';
        }
        if (onResize && containerRef.current) {
            requestAnimationFrame(() => {
                if (containerRef.current) {
                    onResize(containerRef.current.offsetHeight);
                }
            });
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [content, isMinimized, color, theme]);

    // ---- Drag handling (GripVertical) ----
    const handleDragPointerDown = useCallback((e: React.PointerEvent) => {
        e.stopPropagation();
        e.preventDefault();
        setIsDragging(true);
        dragStartYRef.current = e.clientY;
        onDragStart?.(id);
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
    }, [id, onDragStart]);

    const handleDragPointerMove = useCallback((e: React.PointerEvent) => {
        if (!isDragging) return;
        e.stopPropagation();
        const deltaY = e.clientY - dragStartYRef.current;
        const deltaLines = Math.round(deltaY / editorLineHeightRef.current);
        if (deltaLines !== 0) {
            onDragMove?.(deltaLines);
            // Reset start so we send incremental deltas
            dragStartYRef.current = e.clientY;
        }
    }, [isDragging, onDragMove]);

    const handleDragPointerUp = useCallback((e: React.PointerEvent) => {
        if (!isDragging) return;
        e.stopPropagation();
        setIsDragging(false);
        onDragEnd?.();
        try { (e.target as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* noop */ }
    }, [isDragging, onDragEnd]);

    // ---- Actions ----
    const toggleMinimized = useCallback(() => {
        suppressBlurRef.current = true;
        const newState = !isMinimized;
        setIsMinimized(newState);
        onChange(id, content, color, newState);
        // Reset suppress after the click cycle
        setTimeout(() => { suppressBlurRef.current = false; }, 0);
    }, [id, content, color, isMinimized, onChange]);

    const handleDelete = useCallback(() => {
        onDelete(id);
    }, [id, onDelete]);

    const handleColorChange = useCallback((c: NoteColorKey) => {
        setColor(c);
        onChange(id, content, c, isMinimized);
    }, [id, content, isMinimized, onChange]);

    const handleBlur = useCallback(() => {
        if (suppressBlurRef.current) return;
        onChange(id, content, color, isMinimized);
    }, [id, content, color, isMinimized, onChange]);

    const resolvedBg = resolveNoteColor(color, theme);
    const textColor = getReadableTextColor(resolvedBg);
    const isLightText = textColor === '#ffffff';

    // ---- Minimized state ----
    if (isMinimized) {
        return (
            <div
                ref={containerRef}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                    e.stopPropagation();
                    toggleMinimized();
                }}
                className="flex items-center gap-1.5 px-2.5 py-1 rounded-md border cursor-pointer transition-all duration-150 select-none hover:shadow-md"
                style={{
                    backgroundColor: resolvedBg,
                    borderColor: isLightText ? 'rgba(255,255,255,0.15)' : 'rgba(0,0,0,0.1)',
                    maxWidth: '300px',
                    color: textColor,
                }}
            >
                <GripVertical size={11} className="opacity-40 flex-shrink-0" style={{ color: textColor }} />
                <span className="truncate text-xs font-medium flex-1" style={{ color: textColor }}>
                    {content || '(empty note)'}
                </span>
                <Maximize2 size={11} className="opacity-50 flex-shrink-0" style={{ color: textColor }} />
            </div>
        );
    }

    // ---- Expanded state ----
    return (
        <div
            ref={containerRef}
            onPointerDown={(e) => e.stopPropagation()}
            className="relative rounded-lg border overflow-hidden transition-all duration-200 group"
            style={{
                backgroundColor: resolvedBg,
                borderColor: isLightText ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)',
                width: '100%',
                maxWidth: '560px',
                boxShadow: theme === 'dark'
                    ? '0 4px 12px rgba(0,0,0,0.3)'
                    : '0 2px 8px rgba(0,0,0,0.12)',
            }}
        >
            {/* Toolbar */}
            <div
                className="flex items-center justify-between px-2 py-1 border-b transition-opacity duration-150"
                style={{
                    borderColor: isLightText ? 'rgba(255,255,255,0.1)' : 'rgba(0,0,0,0.06)',
                    background: isLightText ? 'rgba(255,255,255,0.08)' : 'rgba(0,0,0,0.04)',
                }}
            >
                {/* Drag handle + color dots */}
                <div className="flex items-center gap-1.5">
                    <div
                        onPointerDown={handleDragPointerDown}
                        onPointerMove={handleDragPointerMove}
                        onPointerUp={handleDragPointerUp}
                        className="cursor-grab active:cursor-grabbing flex items-center justify-center p-0.5 rounded hover:bg-black/10 transition-colors"
                        style={{ touchAction: 'none' }}
                        title="Drag to reposition"
                    >
                        <GripVertical
                            size={13}
                            style={{ color: textColor, opacity: 0.5 }}
                        />
                    </div>
                    <div className="w-px h-3" style={{ background: isLightText ? 'rgba(255,255,255,0.15)' : 'rgba(0,0,0,0.1)' }} />
                    <div className="flex items-center gap-1">
                        {NOTE_COLOR_KEYS.map(k => {
                            const c = resolveNoteColor(k, theme);
                            return (
                                <button
                                    key={k}
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        handleColorChange(k);
                                    }}
                                    onPointerDown={(e) => e.stopPropagation()}
                                    className="rounded-full transition-transform hover:scale-125"
                                    style={{
                                        width: 11,
                                        height: 11,
                                        backgroundColor: c,
                                        outline: color === k ? `2px solid ${textColor}` : 'none',
                                        outlineOffset: 1,
                                        opacity: color === k ? 1 : 0.6,
                                    }}
                                    title={k}
                                />
                            );
                        })}
                    </div>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-0.5">
                    <button
                        onPointerDown={(e) => {
                            e.stopPropagation();
                            e.preventDefault();
                            suppressBlurRef.current = true;
                            setTimeout(() => { suppressBlurRef.current = false; }, 0);
                        }}
                        onClick={(e) => {
                            e.stopPropagation();
                            toggleMinimized();
                        }}
                        className="p-1 rounded transition-colors hover:bg-black/15"
                        style={{ color: textColor }}
                        title="Minimize"
                    >
                        <Minimize2 size={13} />
                    </button>
                    <button
                        onPointerDown={(e) => {
                            e.stopPropagation();
                            e.preventDefault();
                            suppressBlurRef.current = true;
                            setTimeout(() => { suppressBlurRef.current = false; }, 0);
                        }}
                        onClick={(e) => {
                            e.stopPropagation();
                            handleDelete();
                        }}
                        className="p-1 rounded transition-colors hover:bg-red-500/20"
                        style={{ color: textColor }}
                        title="Delete note"
                    >
                        <Trash2 size={13} />
                    </button>
                </div>
            </div>

            {/* Content */}
            <div className="px-3 py-2.5">
                <textarea
                    ref={textareaRef}
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    onBlur={handleBlur}
                    onPointerDown={(e) => e.stopPropagation()}
                    placeholder="Write a note..."
                    className="w-full bg-transparent border-none resize-none focus:outline-none text-sm leading-relaxed placeholder-opacity-40"
                    style={{
                        color: textColor,
                        minHeight: '48px',
                        caretColor: textColor,
                        // Use system font stack, not Comic Sans
                        fontFamily: 'inherit',
                    }}
                />
            </div>
        </div>
    );
};
