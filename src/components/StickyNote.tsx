import { useState, useEffect, useRef } from 'react';
import { Minimize2, Maximize2, Trash2 } from 'lucide-react';

export interface StickyNoteProps {
    id: string;
    initialContent: string;
    initialColor: string;
    initialMinimized?: boolean;
    onChange: (id: string, content: string, color: string, minimized: boolean) => void;
    onDelete: (id: string) => void;
    onResize?: (height: number) => void;
}

const COLORS = [
    '#fff9c4', // Yellow (Classic)
    '#b3e5fc', // Light Blue
    '#c8e6c9', // Light Green
    '#ffccbc', // Light Orange
    '#f8bbd0', // Light Pink
    '#e1bee7', // Light Purple
];

export const StickyNote: React.FC<StickyNoteProps> = ({
    id,
    initialContent,
    initialColor,
    initialMinimized,
    onChange,
    onDelete,
    onResize,
}) => {
    const [content, setContent] = useState(initialContent);
    const [color, setColor] = useState(initialColor || COLORS[0]);
    // Allow local toggle but sync with parent
    const [isMinimized, setIsMinimized] = useState(!!initialMinimized);

    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);

    // Auto-resize textarea & notify parent
    useEffect(() => {
        if (textareaRef.current && !isMinimized) {
            textareaRef.current.style.height = 'auto';
            textareaRef.current.style.height = textareaRef.current.scrollHeight + 'px';
        }

        // Notify parent of new height (for ViewZone resizing)
        if (onResize && containerRef.current) {
            // Give a small delay for layout to settle?
            requestAnimationFrame(() => {
                if (containerRef.current) {
                    onResize(containerRef.current.offsetHeight);
                }
            });
        }
    }, [content, isMinimized]);

    const handleSave = () => {
        onChange(id, content, color, isMinimized);
    };

    const handleDelete = () => {
        onDelete(id);
    };

    const toggleMinimized = () => {
        const newState = !isMinimized;
        setIsMinimized(newState);
        onChange(id, content, color, newState);
    };

    if (isMinimized) {
        return (
            <div
                onMouseDown={(e) => e.stopPropagation()}
                className="flex items-center gap-2 px-3 py-1 rounded shadow-sm border border-gray-300 cursor-pointer hover:shadow-md transition-all select-none"
                style={{ backgroundColor: color, maxWidth: '300px' }}
                onClick={(e) => {
                    e.stopPropagation();
                    toggleMinimized();
                }}
            >
                <span className="truncate text-xs font-medium text-gray-700 flex-1">
                    {content || '(Empty Note)'}
                </span>
                <Maximize2 size={12} className="text-gray-500" />
            </div>
        );
    }

    return (
        <div
            ref={containerRef}
            className="relative rounded-md shadow-md border border-gray-200/50 group transition-all duration-200 overflow-hidden"
            style={{
                backgroundColor: color,
                width: '100%',
                maxWidth: '600px',
                zIndex: 50, // Ensure above Monaco lines
                fontFamily: '"Comic Sans MS", "Chalkboard SE", "Marker Felt", sans-serif' // Handwritten feel
            }}
        >
            {/* Header / Toolbar */}
            <div
                onMouseDown={(e) => {
                    // Prevent Monaco from stealing focus when clicking header
                    e.stopPropagation();
                }}
                className="flex items-center justify-between px-2 py-1 border-b border-black/5 bg-black/5 opacity-0 group-hover:opacity-100 transition-opacity"
            >
                <div className="flex items-center gap-1">
                    {COLORS.map(c => (
                        <button
                            key={c}
                            className={`w-3 h-3 rounded-full border border-black/10 hover:scale-125 transition-transform ${color === c ? 'ring-1 ring-offset-1 ring-gray-400' : ''}`}
                            style={{ backgroundColor: c }}
                            onClick={(e) => {
                                e.stopPropagation();
                                setColor(c);
                                onChange(id, content, c, isMinimized);
                            }}
                            onMouseDown={(e) => e.stopPropagation()}
                            title="Change Color"
                        />
                    ))}
                </div>
                <div className="flex items-center gap-1">
                    <button
                        onClick={(e) => {
                            e.stopPropagation();
                            toggleMinimized();
                        }}
                        onMouseDown={(e) => e.stopPropagation()}
                        className="p-1 hover:bg-black/10 rounded text-gray-600"
                        title="Minimize"
                    >
                        <Minimize2 size={14} />
                    </button>
                    <button
                        onClick={(e) => {
                            e.stopPropagation();
                            handleDelete();
                        }}
                        onMouseDown={(e) => e.stopPropagation()}
                        className="p-1 hover:bg-red-500/20 hover:text-red-700 rounded text-gray-600"
                        title="Delete Note"
                    >
                        <Trash2 size={14} />
                    </button>
                </div>
            </div>

            {/* Content Area */}
            <div className="p-3">
                <textarea
                    ref={textareaRef}
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    onBlur={handleSave}
                    placeholder="Write a note..."
                    className="w-full bg-transparent border-none resize-none focus:ring-0 text-gray-800 placeholder-gray-500/50 text-sm leading-relaxed"
                    style={{ minHeight: '60px' }}
                />
            </div>
        </div>
    );
};
