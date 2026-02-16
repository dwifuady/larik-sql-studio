import { useEffect, useState, useRef } from 'react';
import { MonacoPreview } from './MonacoPreview';
import { formatCellContent } from '../utils/cellFormatter';
import { type CellValue } from '../types';

interface CellPreviewPanelProps {
  visible: boolean;
  width: number;
  selectedCell: {
    tabId: string;
    resultIndex: number;
    rowIndex: number;
    colIndex: number;
    value: CellValue;
    columnName: string;
    dataType: string;
  } | null;
  formatterType: 'auto' | 'json' | 'xml' | 'plain';
  onClose: () => void;
  onResize: (width: number) => void;
  onResizeImmediate: (width: number) => void;
  onFormatChange: (formatter: 'auto' | 'json' | 'xml' | 'plain') => void;
}

// Reuse getTypeColor from ResultsGrid
function getTypeColor(dataType: string): string {
  const type = dataType.toLowerCase();
  if (type.includes('int') || type.includes('numeric') || type.includes('decimal') || type.includes('float') || type.includes('real') || type.includes('money')) {
    return 'text-blue-400';
  }
  if (type.includes('char') || type.includes('text') || type.includes('varchar') || type.includes('nvarchar')) {
    return 'text-green-400';
  }
  if (type.includes('date') || type.includes('time')) {
    return 'text-purple-400';
  }
  if (type.includes('bit') || type.includes('bool')) {
    return 'text-yellow-400';
  }
  if (type.includes('binary') || type.includes('image') || type.includes('varbinary')) {
    return 'text-orange-400';
  }
  if (type.includes('xml')) {
    return 'text-pink-400';
  }
  if (type.includes('uniqueidentifier') || type.includes('uuid')) {
    return 'text-cyan-400';
  }
  return 'text-[var(--text-secondary)]';
}

export function CellPreviewPanel({
  visible,
  width,
  selectedCell,
  formatterType,
  onClose,
  onResize,
  onResizeImmediate,
  onFormatChange
}: CellPreviewPanelProps) {
  const [isResizing, setIsResizing] = useState(false);
  const [isFormatDropdownOpen, setIsFormatDropdownOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const formatDropdownRef = useRef<HTMLDivElement>(null);

  // Handle resize
  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    console.log('[CellPreview] Resize handle mouseDown', e.clientX);
    setIsResizing(true);
  };

  useEffect(() => {
    if (!isResizing) return;

    console.log('[CellPreview] Starting resize');

    let rafId: number | null = null;
    let pendingWidth: number | null = null;

    const handleMouseMove = (e: MouseEvent) => {
      const container = panelRef.current?.parentElement;
      if (!container) return;

      // Get container's position relative to viewport
      const containerRect = container.getBoundingClientRect();

      // Calculate new width as distance from mouse to right edge of container
      pendingWidth = containerRect.right - e.clientX;

      // Throttle updates using requestAnimationFrame
      if (rafId === null) {
        rafId = requestAnimationFrame(() => {
          if (pendingWidth !== null) {
            console.log('[CellPreview] Resizing:', {
              containerRight: containerRect.right,
              clientX: e.clientX,
              newWidth: pendingWidth
            });

            // Use immediate version during resize (no localStorage save)
            onResizeImmediate(pendingWidth);
            pendingWidth = null;
          }
          rafId = null;
        });
      }
    };

    const handleMouseUp = () => {
      console.log('[CellPreview] Resize ended');

      // Apply any pending resize immediately on mouse up
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      if (pendingWidth !== null) {
        // Save to localStorage on mouse up
        onResize(pendingWidth);
        pendingWidth = null;
      }

      setIsResizing(false);
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing, onResize, onResizeImmediate]);

  // Handle keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };

    if (visible) {
      document.addEventListener('keydown', handleKeyDown);
    }

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [visible, onClose]);

  // Close format dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (formatDropdownRef.current && !formatDropdownRef.current.contains(e.target as Node)) {
        setIsFormatDropdownOpen(false);
      }
    };

    if (isFormatDropdownOpen) {
      document.addEventListener('mousedown', handleClickOutside);
    }

    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
    };
  }, [isFormatDropdownOpen]);

  if (!visible || !selectedCell) {
    return null;
  }

  // Format the cell content - convert number[] to Uint8Array if needed for binary data
  const formatterValue = Array.isArray(selectedCell.value) && selectedCell.value.every(v => typeof v === 'number') 
    ? new Uint8Array(selectedCell.value) 
    : selectedCell.value as (string | number | boolean | null | Uint8Array);
  const { content, language, error } = formatCellContent(formatterValue, formatterType);

  // Handle special cases
  const isNull = selectedCell.value === null || selectedCell.value === undefined;
  const isEmpty = !isNull && String(selectedCell.value).trim() === '';
  const isLarge = content.length > 100000;

  // Copy to clipboard
  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(content);
      // TODO: Show toast notification
      console.log('Content copied to clipboard');
    } catch (error) {
      console.error('Failed to copy to clipboard:', error);
    }
  };

  const typeColor = getTypeColor(selectedCell.dataType);

  return (
    <div
      ref={panelRef}
      className="flex flex-col h-full border-l border-[var(--border-color)] bg-[var(--bg-primary)] relative"
      style={{ width: `${width}px`, minWidth: '300px' }}
      role="complementary"
      aria-label="Cell preview"
    >
      {/* Resize handle - wider for easier grabbing */}
      <div
        className={`absolute left-0 top-0 bottom-0 cursor-col-resize z-20 ${
          isResizing ? 'bg-[var(--accent-color)]' : 'bg-[var(--accent-color)]/30 hover:bg-[var(--accent-color)]/70'
        } transition-colors`}
        style={{
          width: '8px',
          marginLeft: '-4px', // Center the 8px handle on the border
          pointerEvents: 'all'
        }}
        onMouseDown={handleMouseDown}
        aria-label="Resize preview panel"
        title="Drag to resize"
      />

      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-[var(--border-color)] bg-[var(--bg-secondary)] min-h-[40px]">
        <div className="flex items-center gap-2 flex-1 min-w-0">
          <span className="font-medium text-sm truncate" title={selectedCell.columnName}>
            {selectedCell.columnName}
          </span>
          <span className={`text-xs px-1.5 py-0.5 rounded ${typeColor} bg-[var(--bg-tertiary)]`}>
            {selectedCell.dataType}
          </span>
        </div>

        <div className="flex items-center gap-1">
          {/* Format selector */}
          <div className="relative" ref={formatDropdownRef}>
            <button
              onClick={() => setIsFormatDropdownOpen(!isFormatDropdownOpen)}
              className="px-2 py-1 text-xs rounded hover:bg-[var(--bg-tertiary)] transition-colors flex items-center gap-1"
              title="Select format"
            >
              <span className="capitalize">{formatterType}</span>
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>

            {isFormatDropdownOpen && (
              <div className="absolute right-0 top-full mt-1 w-32 bg-[var(--bg-secondary)] border border-[var(--border-color)] rounded shadow-lg z-20">
                {(['auto', 'json', 'xml', 'plain'] as const).map((format) => (
                  <button
                    key={format}
                    onClick={() => {
                      onFormatChange(format);
                      setIsFormatDropdownOpen(false);
                    }}
                    className={`w-full px-3 py-1.5 text-left text-xs hover:bg-[var(--bg-tertiary)] transition-colors capitalize ${
                      formatterType === format ? 'text-[var(--accent-color)]' : ''
                    }`}
                  >
                    {format}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Copy button */}
          <button
            onClick={handleCopy}
            className="p-1.5 rounded hover:bg-[var(--bg-tertiary)] transition-colors"
            title="Copy to clipboard"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
            </svg>
          </button>

          {/* Close button */}
          <button
            onClick={onClose}
            className="p-1.5 rounded hover:bg-[var(--bg-tertiary)] transition-colors"
            title="Close preview (Esc)"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-hidden relative">
        {/* Warning banner for errors */}
        {error && (
          <div className="absolute top-0 left-0 right-0 z-10 px-4 py-2 bg-yellow-500/10 border-b border-yellow-500/30 text-yellow-400 text-xs">
            {error}
          </div>
        )}

        {/* NULL value */}
        {isNull && (
          <div className="flex items-center justify-center h-full text-[var(--text-secondary)]">
            <div className="text-center">
              <div className="text-4xl mb-2">∅</div>
              <div className="text-sm">NULL value</div>
            </div>
          </div>
        )}

        {/* Empty string */}
        {!isNull && isEmpty && (
          <div className="flex items-center justify-center h-full text-[var(--text-secondary)]">
            <div className="text-center">
              <div className="text-2xl mb-2 font-mono">""</div>
              <div className="text-sm">Empty string</div>
            </div>
          </div>
        )}

        {/* Large content warning */}
        {!isNull && !isEmpty && isLarge && (
          <div className="absolute top-0 left-0 right-0 z-10 px-4 py-2 bg-orange-500/10 border-b border-orange-500/30 text-orange-400 text-xs">
            Large content ({Math.round(content.length / 1024)} KB) - may affect performance
          </div>
        )}

        {/* Monaco editor for normal content */}
        {!isNull && !isEmpty && (
          <div className={`h-full ${error || isLarge ? 'pt-8' : ''}`}>
            <MonacoPreview content={content} language={language} />
          </div>
        )}
      </div>
    </div>
  );
}
