// Virtual scrolling results grid component (T021, T022)
// Handles 100,000+ rows at 60 FPS using react-window v2
import { useCallback, useMemo, useRef, useState, useEffect, memo, type ReactElement, type CSSProperties } from 'react';
import { List } from 'react-window';
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd';
import { ContextMenu } from './ContextMenu';
import { ExportButton } from './ExportDialog';
import { CellPreviewPanel } from './CellPreviewPanel';
import { useAppStore } from '../store';
import type { QueryResult, ColumnInfo, CellValue } from '../types';

interface ResultsGridProps {
  result: QueryResult;
  onClose: () => void;
  isExecuting?: boolean;
  spaceColor?: string;
  /** Callback to execute an UPDATE query for saving edits */
  onExecuteUpdate?: (query: string) => Promise<boolean>;
  /** Whether the grid is connected and can save edits */
  canEdit?: boolean;
  /** The original query text (for extracting table name) */
  queryText?: string;
  /** Tab ID for updating result cells after save */
  tabId?: string;
  /** Result index within the tab's results */
  resultIndex?: number;
}

// Track edited cells: key is "rowIndex-colIndex", value is the new value
interface EditedCell {
  rowIndex: number;
  colIndex: number;
  originalValue: CellValue;
  newValue: CellValue;
}

// Cell being actively edited
interface EditingCell {
  rowIndex: number;
  colIndex: number;
  value: string;
}

// Cell value formatter
function formatCellValue(value: CellValue): React.ReactNode {
  if (value === null) {
    return <span className="text-gray-500 italic">NULL</span>;
  }
  if (typeof value === 'boolean') {
    return (
      <span className={value ? 'text-green-400' : 'text-red-400'}>
        {value ? 'true' : 'false'}
      </span>
    );
  }
  if (typeof value === 'number') {
    return <span className="font-mono">{value}</span>;
  }
  if (Array.isArray(value)) {
    // Binary data
    return <span className="text-gray-500 italic">[binary]</span>;
  }
  // String - truncate if too long for display
  const str = String(value);
  // Show empty strings with a visual indicator
  if (str.length === 0) {
    return <span className="text-gray-500 italic">(empty)</span>;
  }
  if (str.length > 500) {
    return str.substring(0, 500) + '...';
  }
  return str;
}

// Data type badge color
function getTypeColor(dataType: string): string {
  const type = dataType.toLowerCase();
  if (type.includes('int') || type.includes('numeric') || type.includes('decimal') || type.includes('float') || type.includes('real') || type.includes('money')) {
    return 'text-blue-400';
  }
  if (type.includes('char') || type.includes('text') || type.includes('string')) {
    return 'text-green-400';
  }
  if (type.includes('date') || type.includes('time')) {
    return 'text-purple-400';
  }
  if (type.includes('bit') || type.includes('bool')) {
    return 'text-orange-400';
  }
  if (type.includes('binary') || type.includes('image') || type.includes('varbinary')) {
    return 'text-gray-400';
  }
  return 'text-gray-500';
}

// Constants for virtual scrolling
const ROW_HEIGHT = 32;
const HEADER_HEIGHT = 40;
const MIN_COLUMN_WIDTH = 100;
const MAX_COLUMN_WIDTH = 400;

// Calculate column widths based on content (memoized - only depends on data, not container width)
function calculateColumnWidths(columns: ColumnInfo[], rows: CellValue[][]): number[] {
  return columns.map((col, colIdx) => {
    // Start with header width estimation (8px per char + padding)
    let maxWidth = Math.min(col.name.length * 8 + 32, MAX_COLUMN_WIDTH);

    // Sample first 100 rows for width calculation
    const sampleRows = rows.slice(0, 100);
    for (const row of sampleRows) {
      const cellValue = row[colIdx];
      let cellWidth = MIN_COLUMN_WIDTH;

      if (cellValue !== null && cellValue !== undefined) {
        const strValue = String(cellValue);
        // Estimate width: 7px per character + padding
        cellWidth = Math.min(strValue.length * 7 + 24, MAX_COLUMN_WIDTH);
      }

      maxWidth = Math.max(maxWidth, cellWidth);
    }

    return Math.max(maxWidth, MIN_COLUMN_WIDTH);
  });
}

// Adjust widths to fit container (cheap operation - just math)
// Only expand if content is significantly smaller than container (with some padding)
function adjustColumnWidthsToContainer(baseWidths: number[]): number[] {
  // Don't expand columns - let tables be their natural size
  return baseWidths;
}

// Row props for the virtual list
interface RowData {
  rows: CellValue[][];
  columns: ColumnInfo[];
  columnWidths: number[];
  totalWidth: number;
  selectedCell: { row: number; col: number } | null;
  selection: SelectionRange | null;
  copiedCell: { row: number; col: number } | null;
  copiedSelection: SelectionRange | null;
  editingCell: EditingCell | null;
  editedCells: Map<string, EditedCell>;
  canEdit: boolean;
  hoveredCell: { row: number; col: number } | null;
  onCellClick: (row: number, col: number) => void;
  onCellMouseDown: (row: number, col: number, shiftKey: boolean) => void;
  onCellMouseEnter: (row: number, col: number) => void;
  onCellMouseUp: () => void;
  onCellDoubleClick: (row: number, col: number) => void;
  onCellContextMenu: (e: React.MouseEvent, row: number, col: number) => void;
  columnOrder: number[];
  onCopyRow: (row: number) => void;
  onEditChange: (value: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onPreviewCell: (row: number, col: number) => void;
}

// Row component props for react-window v2
interface RowComponentProps {
  ariaAttributes: {
    "aria-posinset": number;
    "aria-setsize": number;
    role: "listitem";
  };
  index: number;
  style: CSSProperties;
  data: RowData;
}

// Manual double-click tracking (survives re-renders)
let lastClickTime = 0;
let lastClickRow = -1;
let lastClickCol = -1;
const DOUBLE_CLICK_DELAY = 300; // ms

// Helper: Is row intersected by selection?
function isRowInSelection(row: number, selection: SelectionRange | null): boolean {
  if (!selection) return false;
  const min = Math.min(selection.startRow, selection.endRow);
  const max = Math.max(selection.startRow, selection.endRow);
  return row >= min && row <= max;
}

// Comparison function to prevent unnecessary row re-renders
function arePropsEqual(prev: RowComponentProps, next: RowComponentProps) {
  if (prev.index !== next.index) return false;

  // If style object changed (scrolling), we must render
  if (prev.style !== next.style) return false;

  const d1 = prev.data;
  const d2 = next.data;
  const idx = next.index;

  if (d1 === d2) return true;

  // 1. Structural/Content checks (global changes)
  if (d1.rows !== d2.rows ||
    d1.columns !== d2.columns ||
    d1.columnOrder !== d2.columnOrder ||
    d1.columnWidths !== d2.columnWidths ||
    d1.totalWidth !== d2.totalWidth ||
    d1.canEdit !== d2.canEdit ||
    d1.editedCells !== d2.editedCells ||
    d1.onCellMouseDown !== d2.onCellMouseDown ||
    d1.onCellClick !== d2.onCellClick) {
    return false;
  }

  // 2. Interaction checks (Row specific)

  // Hover
  // If this row is involved in hover change (either was hovered or is now hovered)
  if ((d1.hoveredCell?.row === idx || d2.hoveredCell?.row === idx) &&
    d1.hoveredCell !== d2.hoveredCell) return false;

  // Selected Cell
  if ((d1.selectedCell?.row === idx || d2.selectedCell?.row === idx) &&
    d1.selectedCell !== d2.selectedCell) return false;

  // Selection Range
  const inSel1 = isRowInSelection(idx, d1.selection);
  const inSel2 = isRowInSelection(idx, d2.selection);
  if (inSel1 !== inSel2) return false;
  if ((inSel1 || inSel2) && d1.selection !== d2.selection) return false;

  // Copied Cell
  if ((d1.copiedCell?.row === idx || d2.copiedCell?.row === idx) &&
    d1.copiedCell !== d2.copiedCell) return false;

  // Copied Selection
  const inCopy1 = isRowInSelection(idx, d1.copiedSelection);
  const inCopy2 = isRowInSelection(idx, d2.copiedSelection);
  if (inCopy1 !== inCopy2) return false;
  if ((inCopy1 || inCopy2) && d1.copiedSelection !== d2.copiedSelection) return false;

  // Editing Cell
  if ((d1.editingCell?.rowIndex === idx || d2.editingCell?.rowIndex === idx) &&
    d1.editingCell !== d2.editingCell) return false;

  return true;
}

// Memoized Cell component to prevent unnecessary re-renders
const Cell = memo(function Cell({
  rowIndex,
  colIndex,
  width,
  cellValue: _cellValue,
  displayValue,
  isSingleSelected,
  isInSelection,
  isCopied,
  isInCopiedSelection,
  isEditing,
  isEdited,
  isHovered,
  editingValue,
  onMouseDown,
  onMouseEnter,
  onMouseUp,
  onContextMenu,
  onEditChange,
  onEditCommit,
  onEditCancel,
  onPreviewCell
}: {
  rowIndex: number;
  colIndex: number;
  width: number;
  cellValue: CellValue;
  displayValue: CellValue;
  isSingleSelected: boolean;
  isInSelection: boolean;
  isCopied: boolean;
  isInCopiedSelection: boolean;
  isEditing: boolean;
  isEdited: boolean;
  isHovered: boolean;
  editingValue: string;
  onMouseDown: (row: number, col: number, shiftKey: boolean) => void;
  onMouseEnter: (row: number, col: number) => void;
  onMouseUp: () => void;
  onContextMenu: (e: React.MouseEvent, row: number, col: number) => void;
  onEditChange: (value: string) => void;
  onEditCommit: () => void;
  onEditCancel: () => void;
  onPreviewCell: (row: number, col: number) => void;
}) {
  return (
    <div
      className={`px-3 flex items-center border-r border-[var(--border-color)] truncate cursor-pointer transition-colors select-none relative ${isSingleSelected || isInSelection ? 'bg-[var(--accent-glow)] ring-1 ring-[var(--accent-color)]' : ''
        } ${isCopied || isInCopiedSelection ? 'bg-green-500/20' : ''} ${isEdited && !isEditing ? 'bg-yellow-500/20' : ''}`}
      style={{ width, minWidth: width, height: ROW_HEIGHT }}
      onMouseDown={(e) => {
        // Only handle left mouse button for selection
        if (e.button !== 0 || isEditing) return;
        onMouseDown(rowIndex, colIndex, e.shiftKey);
      }}
      onMouseEnter={() => !isEditing && onMouseEnter(rowIndex, colIndex)}
      onMouseUp={() => !isEditing && onMouseUp()}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!isEditing) onContextMenu(e, rowIndex, colIndex);
      }}
      title={isEditing ? undefined : (displayValue === null ? 'NULL' : String(displayValue))}
    >
      {isEditing ? (
        <input
          type="text"
          className="w-full h-6 px-1 text-xs bg-[var(--bg-primary)] border border-[var(--accent-color)] rounded outline-none text-[var(--text-primary)]"
          value={editingValue}
          onChange={(e) => onEditChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              onEditCommit();
            } else if (e.key === 'Escape') {
              e.preventDefault();
              onEditCancel();
            }
          }}
          autoFocus
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
        />
      ) : (
        <>
          <span className={`truncate text-xs text-[var(--text-primary)] ${isEdited ? 'font-medium' : ''}`}>
            {formatCellValue(displayValue)}
          </span>
          {isHovered && !isEditing && (
            <button
              onClick={(e) => {
                e.stopPropagation();
                e.preventDefault();
                onPreviewCell(rowIndex, colIndex);
              }}
              onMouseDown={(e) => {
                e.stopPropagation();
                e.preventDefault();
              }}
              onMouseUp={(e) => {
                e.stopPropagation();
              }}
              onMouseEnter={(e) => {
                e.stopPropagation();
              }}
              className="absolute right-1 p-0.5 rounded bg-[var(--bg-secondary)] hover:bg-[var(--accent-color)] transition-colors z-10 cursor-pointer"
              title="Preview cell content (Space)"
            >
              <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 8V4m0 0h4M4 4l5 5m11-1V4m0 0h-4m4 0l-5 5M4 16v4m0 0h4m-4 0l5-5m11 5l-5-5m5 5v-4m0 4h-4" />
              </svg>
            </button>
          )}
        </>
      )}
    </div>
  );
});

const Row = memo(function Row({
  index,
  style,
  data
}: RowComponentProps): ReactElement | null {
  const { rows, columnWidths, totalWidth, selectedCell, selection, copiedCell, copiedSelection, editingCell, editedCells, hoveredCell, onCellMouseDown, onCellMouseEnter, onCellMouseUp, onCellContextMenu, onCopyRow, onEditChange, onEditCommit, onEditCancel, onPreviewCell, columnOrder } = data;

  const row = rows[index];
  if (!row) return null;

  // Helper to get edited key
  const getEditKey = (rowIdx: number, colIdx: number) => `${rowIdx}-${colIdx}`;

  return (
    <div
      style={{ ...style, width: totalWidth, minWidth: totalWidth }}
      className={`flex items-center border-b border-[var(--border-color)] ${index % 2 === 0 ? 'bg-transparent' : 'bg-[var(--bg-hover)]'
        } hover:bg-[var(--bg-active)] group`}
    >
      {/* Row number */}
      <div
        className="shrink-0 w-12 px-2 text-xs text-[var(--text-muted)] border-r border-[var(--border-color)] h-full flex items-center justify-center relative"
        style={{ minWidth: 48 }}
      >
        <span className="group-hover:opacity-0 transition-opacity">{index + 1}</span>
        <button
          onClick={(e) => { e.stopPropagation(); onCopyRow(index); }}
          className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity text-[var(--text-muted)] hover:text-[var(--text-primary)]"
          title="Copy row as JSON"
        >
          <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
          </svg>
        </button>
      </div>

      {/* Cells rendered in columnOrder */}
      {columnOrder.map((colIdx) => {
        const cell = row[colIdx];
        const isSingleSelected = selectedCell?.row === index && selectedCell?.col === colIdx;
        const isInSelection = isCellInSelection(index, colIdx, selection, columnOrder);
        const isCopied = copiedCell?.row === index && copiedCell?.col === colIdx;
        const isInCopiedSelection = isCellInSelection(index, colIdx, copiedSelection, columnOrder);
        const isEditing = editingCell?.rowIndex === index && editingCell?.colIndex === colIdx;
        const editKey = getEditKey(index, colIdx);
        const isEdited = editedCells.has(editKey);
        const isHovered = hoveredCell?.row === index && hoveredCell?.col === colIdx;

        // Get the display value - use edited value if this cell was edited
        const displayValue = isEdited ? editedCells.get(editKey)!.newValue : cell;

        return (
          <Cell
            key={colIdx}
            rowIndex={index}
            colIndex={colIdx}
            width={columnWidths[colIdx]}
            cellValue={cell}
            displayValue={displayValue}
            isSingleSelected={isSingleSelected}
            isInSelection={isInSelection}
            isCopied={isCopied}
            isInCopiedSelection={isInCopiedSelection}
            isEditing={isEditing}
            isEdited={isEdited}
            isHovered={isHovered}
            editingValue={editingCell?.value || ''}
            onMouseDown={onCellMouseDown}
            onMouseEnter={onCellMouseEnter}
            onMouseUp={onCellMouseUp}
            onContextMenu={onCellContextMenu}
            onEditChange={onEditChange}
            onEditCommit={onEditCommit}
            onEditCancel={onEditCancel}
            onPreviewCell={onPreviewCell}
          />
        );
      })}
    </div>
  );
}, arePropsEqual);

// Selection range type for multi-cell selection
interface SelectionRange {
  startRow: number;
  startCol: number;
  endRow: number;
  endCol: number;
}

// Get normalized selection (ensure start <= end)
function normalizeSelection(selection: SelectionRange): SelectionRange {
  return {
    startRow: Math.min(selection.startRow, selection.endRow),
    startCol: Math.min(selection.startCol, selection.endCol),
    endRow: Math.max(selection.startRow, selection.endRow),
    endCol: Math.max(selection.startCol, selection.endCol),
  };
}

// Check if a cell is within a selection range, respecting visual column order
function isCellInSelection(row: number, col: number, selection: SelectionRange | null, columnOrder?: number[]): boolean {
  if (!selection) return false;
  const norm = normalizeSelection(selection);

  const inRowRange = row >= norm.startRow && row <= norm.endRow;
  if (!inRowRange) return false;

  if (columnOrder) {
    const visualCol = columnOrder.indexOf(col);
    const visualStart = columnOrder.indexOf(selection.startCol);
    const visualEnd = columnOrder.indexOf(selection.endCol);
    const minVisual = Math.min(visualStart, visualEnd);
    const maxVisual = Math.max(visualStart, visualEnd);
    return visualCol >= minVisual && visualCol <= maxVisual;
  }

  return col >= norm.startCol && col <= norm.endCol;
}

// Format a cell value for SQL INSERT statement
function formatValueForInsert(value: CellValue, dataType: string): string {
  if (value === null) {
    return 'NULL';
  }
  if (typeof value === 'boolean') {
    return value ? '1' : '0';
  }
  if (typeof value === 'number') {
    return String(value);
  }
  if (Array.isArray(value)) {
    // Binary data - convert to hex string
    const hex = value.map(b => b.toString(16).padStart(2, '0')).join('');
    return `0x${hex}`;
  }
  // String value - escape single quotes
  const type = dataType.toLowerCase();
  const strValue = String(value).replace(/'/g, "''");

  // For date/time types, use appropriate format
  if (type.includes('date') || type.includes('time')) {
    return `'${strValue}'`;
  }

  // For numeric types stored as string, don't quote
  if ((type.includes('int') || type.includes('numeric') || type.includes('decimal') ||
    type.includes('float') || type.includes('real') || type.includes('money')) &&
    !isNaN(Number(strValue))) {
    return strValue;
  }

  // String - wrap with N' for nvarchar/nchar support
  if (type.includes('nvarchar') || type.includes('nchar') || type.includes('ntext')) {
    return `N'${strValue}'`;
  }

  return `'${strValue}'`;
}

function ResultsGridComp({ result, onClose, isExecuting = false, spaceColor = '#6366f1', onExecuteUpdate, canEdit = false, queryText, tabId, resultIndex }: ResultsGridProps) {
  const updateResultCells = useAppStore((state) => state.updateResultCells);
  const resultColumnOrder = useAppStore((state) => state.resultColumnOrder);
  const setResultColumnOrder = useAppStore((state) => state.setResultColumnOrder);

  const cellPreviewPanel = useAppStore(s => s.cellPreviewPanel);
  const showCellPreview = useAppStore(s => s.showCellPreview);
  const hideCellPreview = useAppStore(s => s.hideCellPreview);
  const setCellPreviewWidth = useAppStore(s => s.setCellPreviewWidth);
  const setCellPreviewWidthImmediate = useAppStore(s => s.setCellPreviewWidthImmediate);
  const setCellPreviewFormatter = useAppStore(s => s.setCellPreviewFormatter);
  const containerRef = useRef<HTMLDivElement>(null);
  const headerRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [containerHeight, setContainerHeight] = useState(300);
  const [selectedCell, setSelectedCell] = useState<{ row: number; col: number } | null>(null);
  const [selection, setSelection] = useState<SelectionRange | null>(null);
  const [isSelecting, setIsSelecting] = useState(false);
  const [copiedCell, setCopiedCell] = useState<{ row: number; col: number } | null>(null);
  const [copiedSelection, setCopiedSelection] = useState<SelectionRange | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; row: number; col: number } | null>(null);
  const [hoverColumnIdx, setHoverColumnIdx] = useState<number | null>(null);
  const [hoveredCell, setHoveredCell] = useState<{ row: number; col: number } | null>(null);

  // Scroll position preservation
  const scrollPositionRef = useRef<number>(0);

  // Editing state
  const [editingCell, setEditingCell] = useState<EditingCell | null>(null);
  const [editedCells, setEditedCells] = useState<Map<string, EditedCell>>(new Map());
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Confirmation dialog state
  const [showConfirmDialog, setShowConfirmDialog] = useState(false);
  const [pendingQueries, setPendingQueries] = useState<string[]>([]);

  // Find the identity/primary key column for WHERE clause
  const identityColumn = useMemo(() => {
    // Look for identity columns by common naming patterns
    const identityPatterns = ['id', 'pk', 'key', '_id', 'identity'];
    for (const col of result.columns) {
      const nameLower = col.name.toLowerCase();
      for (const pattern of identityPatterns) {
        if (nameLower === pattern || nameLower.endsWith(pattern)) {
          return { index: result.columns.indexOf(col), name: col.name, dataType: col.data_type };
        }
      }
    }
    // Fallback to first column
    if (result.columns.length > 0) {
      return { index: 0, name: result.columns[0].name, dataType: result.columns[0].data_type };
    }
    return null;
  }, [result.columns]);

  // Extract table name from the query statement
  const tableName = useMemo(() => {
    // Prefer result.statement_text (individual statement from batch) over queryText (full query)
    // For batch queries, result.statement_text contains the specific statement that produced this result
    const stmt = result.statement_text || queryText;
    if (!stmt) return null;

    // Try to extract table name from SELECT ... FROM [schema].[table] or FROM table
    // Handle various formats: FROM table, FROM [table], FROM schema.table, FROM [schema].[table]
    const fromMatch = stmt.match(/\bFROM\s+(\[?[\w]+\]?\.)?(\[?[\w]+\]?)/i);
    if (fromMatch) {
      const schemaAndTable = fromMatch[1] ? fromMatch[1] + fromMatch[2] : fromMatch[2];
      // Clean up and format
      const cleaned = schemaAndTable.replace(/\[|\]/g, '');
      if (cleaned.includes('.')) {
        const [schema, table] = cleaned.split('.');
        return `[${schema}].[${table}]`;
      } else {
        return `[dbo].[${cleaned}]`;
      }
    }
    return null;
  }, [queryText, result.statement_text]);

  // Check if editing is actually possible - allow editing even without table name (will prompt on save)
  // We allow editing UI as long as there are columns (for better UX - can edit, error shown on save)
  const canActuallyEdit = result.columns.length > 0;

  // Check if we can actually save (has connection and update handler)
  const canSaveEdits = canEdit && onExecuteUpdate && identityColumn !== null;

  // Calculate base column widths (expensive - only when data changes)
  const baseColumnWidths = useMemo(() => {
    return calculateColumnWidths(result.columns, result.rows);
  }, [result.columns, result.rows]);

  // Adjust widths to container (cheap - can run more often)
  const columnWidths = useMemo(() => {
    return adjustColumnWidthsToContainer(baseColumnWidths);
  }, [baseColumnWidths]);

  const totalWidth = useMemo(() => {
    return columnWidths.reduce((sum, w) => sum + w, 0) + 48; // +48 for row number column
  }, [columnWidths]);

  // Column order state from store (or default)
  const columnOrder = useMemo(() => {
    if (tabId && resultIndex !== undefined) {
      const stored = resultColumnOrder[tabId]?.[resultIndex];
      if (stored && stored.length === result.columns.length) {
        return stored;
      }
    }
    // Default order [0, 1, 2, ...]
    return result.columns.map((_, i) => i);
  }, [tabId, resultIndex, result.columns.length, resultColumnOrder]);

  const handleDragEnd = useCallback((result: DropResult) => {
    if (!result.destination || !tabId || resultIndex === undefined) return;

    const newOrder = Array.from(columnOrder);
    const [reorderedItem] = newOrder.splice(result.source.index, 1);
    newOrder.splice(result.destination.index, 0, reorderedItem);

    setResultColumnOrder(tabId, resultIndex, newOrder);
  }, [columnOrder, tabId, resultIndex, setResultColumnOrder]);

  // Measure container size with throttling for performance
  useEffect(() => {
    if (!containerRef.current) return;

    let rafId: number | null = null;
    let lastUpdate = 0;
    const THROTTLE_MS = 16; // ~60fps for height, width is debounced separately

    const resizeObserver = new ResizeObserver((entries) => {
      const now = Date.now();

      // Throttle updates during rapid resize
      if (now - lastUpdate < THROTTLE_MS) {
        if (rafId) cancelAnimationFrame(rafId);
        rafId = requestAnimationFrame(() => {
          for (const entry of entries) {
            setContainerHeight(entry.contentRect.height - HEADER_HEIGHT);
          }
          lastUpdate = Date.now();
        });
        return;
      }

      for (const entry of entries) {
        setContainerHeight(entry.contentRect.height - HEADER_HEIGHT);
      }
      lastUpdate = now;
    });

    resizeObserver.observe(containerRef.current);
    return () => {
      if (rafId) cancelAnimationFrame(rafId);
      resizeObserver.disconnect();
    };
  }, []);

  // Sync horizontal scroll between header and body
  useEffect(() => {
    if (!bodyRef.current || !headerRef.current) return;

    let cleanupFn: (() => void) | null = null;

    // Use a slight delay to ensure react-window has rendered
    const timer = setTimeout(() => {
      // Find the actual scrollable element (react-window creates a div with overflow)
      const scrollableElement = bodyRef.current?.querySelector('[style*="overflow"]') as HTMLElement;
      if (!scrollableElement) return;

      // Restore previous scroll position if List was remounted
      if (scrollPositionRef.current > 0) {
        scrollableElement.scrollLeft = scrollPositionRef.current;
        if (headerRef.current) {
          headerRef.current.scrollLeft = scrollPositionRef.current;
        }
      }

      const handleScroll = () => {
        if (headerRef.current) {
          headerRef.current.scrollLeft = scrollableElement.scrollLeft;
          // Store scroll position for preservation across remounts
          scrollPositionRef.current = scrollableElement.scrollLeft;
        }
      };

      // Use passive listener for better scroll performance
      scrollableElement.addEventListener('scroll', handleScroll, { passive: true });

      // Store cleanup function
      cleanupFn = () => scrollableElement.removeEventListener('scroll', handleScroll);
    }, 10);

    return () => {
      clearTimeout(timer);
      cleanupFn?.();
    };
  }, [result.rows.length, editingCell]); // Add editingCell to dependencies

  // Close cell preview panel when results change
  useEffect(() => {
    // Close the preview panel when new results arrive to avoid stale data
    if (cellPreviewPanel.visible) {
      hideCellPreview();
    }
  }, [result.query_id]); // Use query_id to detect new query results

  // Copy cell value to clipboard
  const handleCopyCell = useCallback(async (rowIdx: number, colIdx: number, includeHeaders = false) => {
    const value = result.rows[rowIdx]?.[colIdx];
    let textValue = value === null ? '' : String(value);

    if (includeHeaders) {
      const header = result.columns[colIdx]?.name || '';
      textValue = `${header}\n${textValue}`;
    }

    try {
      await navigator.clipboard.writeText(textValue);
      setCopiedCell({ row: rowIdx, col: colIdx });
      setCopiedSelection(null);
      setTimeout(() => setCopiedCell(null), 1500);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  }, [result.rows, result.columns]);

  // Copy selection (multiple cells) as tab-separated values, respecting visual order
  const handleCopySelection = useCallback(async (sel: SelectionRange, includeHeaders = false) => {
    const norm = normalizeSelection(sel);
    const visualStart = columnOrder.indexOf(sel.startCol);
    const visualEnd = columnOrder.indexOf(sel.endCol);
    const minVisual = Math.min(visualStart, visualEnd);
    const maxVisual = Math.max(visualStart, visualEnd);

    const visualCols: number[] = [];
    for (let i = minVisual; i <= maxVisual; i++) {
      visualCols.push(columnOrder[i]);
    }

    const lines: string[] = [];

    if (includeHeaders) {
      const headers = visualCols.map(colIdx => result.columns[colIdx]?.name || '');
      lines.push(headers.join('\t'));
    }

    for (let row = norm.startRow; row <= norm.endRow; row++) {
      const rowValues: string[] = [];
      for (const col of visualCols) {
        const value = result.rows[row]?.[col];
        rowValues.push(value === null ? '' : String(value));
      }
      lines.push(rowValues.join('\t'));
    }

    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      setCopiedSelection(sel);
      setCopiedCell(null);
      setTimeout(() => setCopiedSelection(null), 1500);
    } catch (err) {
      console.error('Failed to copy selection:', err);
    }
  }, [result.rows, result.columns, columnOrder]);

  // Copy selection as SQL INSERT VALUES, respecting visual order
  const handleCopyAsInsert = useCallback(async (sel: SelectionRange) => {
    const norm = normalizeSelection(sel);
    const visualStart = columnOrder.indexOf(sel.startCol);
    const visualEnd = columnOrder.indexOf(sel.endCol);
    const minVisual = Math.min(visualStart, visualEnd);
    const maxVisual = Math.max(visualStart, visualEnd);

    const visualCols: number[] = [];
    for (let i = minVisual; i <= maxVisual; i++) {
      visualCols.push(columnOrder[i]);
    }

    const valueRows: string[] = [];

    for (let row = norm.startRow; row <= norm.endRow; row++) {
      const rowValues: string[] = [];
      for (const col of visualCols) {
        const value = result.rows[row]?.[col];
        const dataType = result.columns[col]?.data_type || 'varchar';
        rowValues.push(formatValueForInsert(value, dataType));
      }
      valueRows.push(`(${rowValues.join(', ')})`);
    }

    // Generate the VALUES clause
    const insertValues = `VALUES ${valueRows.join(',\n       ')}`;

    try {
      await navigator.clipboard.writeText(insertValues);
      setCopiedSelection(sel);
      setCopiedCell(null);
      setTimeout(() => setCopiedSelection(null), 1500);
    } catch (err) {
      console.error('Failed to copy as INSERT:', err);
    }
  }, [result.rows, result.columns, columnOrder]);

  // Copy selection as SQL IN clause, respecting visual order
  const handleCopyAsInClause = useCallback(async (sel: SelectionRange) => {
    const norm = normalizeSelection(sel);
    const visualStart = columnOrder.indexOf(sel.startCol);
    const visualEnd = columnOrder.indexOf(sel.endCol);
    const minVisual = Math.min(visualStart, visualEnd);
    const maxVisual = Math.max(visualStart, visualEnd);

    const visualCols: number[] = [];
    for (let i = minVisual; i <= maxVisual; i++) {
      visualCols.push(columnOrder[i]);
    }

    const values: string[] = [];

    // For IN clause, we collect all values from the selection
    for (let row = norm.startRow; row <= norm.endRow; row++) {
      for (const col of visualCols) {
        const value = result.rows[row]?.[col];
        const dataType = result.columns[col]?.data_type || 'varchar';
        values.push(formatValueForInsert(value, dataType));
      }
    }

    // Generate the IN clause
    const inClause = `IN (${values.join(', ')})`;

    try {
      await navigator.clipboard.writeText(inClause);
      setCopiedSelection(sel);
      setCopiedCell(null);
      setTimeout(() => setCopiedSelection(null), 1500);
    } catch (err) {
      console.error('Failed to copy as IN clause:', err);
    }
  }, [result.rows, result.columns, columnOrder]);

  // Preview cell handler
  const handlePreviewCell = useCallback((row: number, col: number) => {
    if (!tabId || resultIndex === undefined) return;

    const value = result.rows[row]?.[col];
    const columnName = result.columns[col]?.name || '';
    const dataType = result.columns[col]?.data_type || '';

    showCellPreview(tabId, resultIndex, row, col, value, columnName, dataType);
  }, [tabId, resultIndex, result.rows, result.columns, showCellPreview]);

  // Select all cells in the current grid
  const handleSelectAll = useCallback(() => {
    if (result.rows.length === 0 || result.columns.length === 0) return;

    setSelection({
      startRow: 0,
      startCol: columnOrder[0],
      endRow: result.rows.length - 1,
      endCol: columnOrder[columnOrder.length - 1],
    });
    setSelectedCell({ row: 0, col: columnOrder[0] });
    containerRef.current?.focus();
  }, [result.rows.length, result.columns.length, columnOrder]);

  // Keyboard shortcut: Ctrl+C to copy selected cell(s) - only when focus is in results grid
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      // Space key - open cell preview
      if (e.key === ' ' && !e.ctrlKey && !e.metaKey && selectedCell) {
        e.preventDefault();
        handlePreviewCell(selectedCell.row, selectedCell.col);
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'c') {
        // Copy selection if multi-cell selection exists
        if (selection) {
          e.preventDefault();
          handleCopySelection(selection, e.shiftKey);
        } else if (selectedCell) {
          e.preventDefault();
          handleCopyCell(selectedCell.row, selectedCell.col, e.shiftKey);
        }
      }

      // Select All: Ctrl+A
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
        const target = e.target as HTMLElement;
        // Don't interfere with inputs inside the grid (like editing cell)
        if (target.tagName !== 'INPUT' && target.tagName !== 'TEXTAREA') {
          e.preventDefault();
          handleSelectAll();
        }
      }
    };

    // Attach to container instead of window so it only fires when grid has focus
    container.addEventListener('keydown', handleKeyDown);
    return () => container.removeEventListener('keydown', handleKeyDown);
  }, [selectedCell, selection, handleCopyCell, handleCopySelection, handlePreviewCell, handleSelectAll]);

  // Copy a single row as JSON, respecting visual column order
  const handleCopyRow = useCallback(async (rowIdx: number) => {
    const row = result.rows[rowIdx];
    if (!row) return;

    const rowObj: Record<string, CellValue> = {};
    columnOrder.forEach((origIdx) => {
      const col = result.columns[origIdx];
      if (col) {
        rowObj[col.name] = row[origIdx];
      }
    });

    try {
      await navigator.clipboard.writeText(JSON.stringify(rowObj, null, 2));
      setCopiedCell({ row: rowIdx, col: -1 }); // Special col index for full row
      setTimeout(() => setCopiedCell(null), 1500);
    } catch (err) {
      console.error('Failed to copy row:', err);
    }
  }, [result.rows, result.columns, columnOrder]);

  // ========== EDITING HANDLERS ==========

  // Start editing a cell on double-click (or copy if editing not possible)
  const handleStartEdit = useCallback((rowIdx: number, colIdx: number) => {
    // Don't allow editing if no columns
    if (!canActuallyEdit) {
      return;
    }

    const cellKey = `${rowIdx}-${colIdx}`;
    const edited = editedCells.get(cellKey);
    const currentValue = edited ? edited.newValue : result.rows[rowIdx]?.[colIdx];

    setEditingCell({
      rowIndex: rowIdx,
      colIndex: colIdx,
      value: currentValue === null ? '' : String(currentValue),
    });
  }, [canActuallyEdit, editedCells, result.rows]);

  // Handle edit input change
  const handleEditChange = useCallback((value: string) => {
    setEditingCell(prev => prev ? { ...prev, value } : null);
  }, []);

  // Commit the edit
  const handleEditCommit = useCallback(() => {
    if (!editingCell) return;

    const { rowIndex, colIndex, value } = editingCell;
    const originalValue = result.rows[rowIndex]?.[colIndex];
    const cellKey = `${rowIndex}-${colIndex}`;

    // Parse the new value based on original type
    let newValue: CellValue;
    if (value === '' || value.toLowerCase() === 'null') {
      newValue = null;
    } else if (typeof originalValue === 'number') {
      newValue = Number(value);
      if (isNaN(newValue)) newValue = value; // Keep as string if not valid number
    } else if (typeof originalValue === 'boolean') {
      newValue = value === '1' || value.toLowerCase() === 'true';
    } else {
      newValue = value;
    }

    // Check if the value actually changed from original
    const existingEdit = editedCells.get(cellKey);
    const wasOriginallyValue = existingEdit ? existingEdit.originalValue : originalValue;

    if (newValue === wasOriginallyValue || (newValue === null && wasOriginallyValue === null) ||
      (String(newValue) === String(wasOriginallyValue))) {
      // Value is same as original, remove from edited cells if it was there
      if (existingEdit) {
        setEditedCells(prev => {
          const next = new Map(prev);
          next.delete(cellKey);
          return next;
        });
      }
    } else {
      // Value changed, add/update in edited cells
      setEditedCells(prev => {
        const next = new Map(prev);
        next.set(cellKey, {
          rowIndex,
          colIndex,
          originalValue: wasOriginallyValue,
          newValue,
        });
        return next;
      });
    }

    setEditingCell(null);
    setSaveError(null);
  }, [editingCell, result.rows, editedCells]);

  // Cancel editing
  const handleEditCancel = useCallback(() => {
    setEditingCell(null);
  }, []);

  // Discard all edits
  const handleDiscardEdits = useCallback(() => {
    setEditedCells(new Map());
    setEditingCell(null);
    setSaveError(null);
  }, []);

  // Generate UPDATE queries for preview
  const generateUpdateQueries = useCallback((): string[] | null => {
    if (editedCells.size === 0) return null;

    if (!identityColumn || !tableName) return null;

    const queries: string[] = [];

    // Group edits by row for efficient UPDATE statements
    const editsByRow = new Map<number, EditedCell[]>();
    for (const edit of editedCells.values()) {
      const rowEdits = editsByRow.get(edit.rowIndex) || [];
      rowEdits.push(edit);
      editsByRow.set(edit.rowIndex, rowEdits);
    }

    // Generate UPDATE for each modified row
    for (const [rowIndex, rowEdits] of editsByRow) {
      const identityValue = result.rows[rowIndex]?.[identityColumn.index];
      if (identityValue === null || identityValue === undefined) continue;

      const setClauses = rowEdits.map(edit => {
        const colName = result.columns[edit.colIndex].name;
        const dataType = result.columns[edit.colIndex].data_type;
        const formattedValue = formatValueForInsert(edit.newValue, dataType);
        return `[${colName}] = ${formattedValue}`;
      });

      const identityFormatted = formatValueForInsert(identityValue, identityColumn.dataType);
      const updateQuery = `UPDATE ${tableName} SET ${setClauses.join(', ')} WHERE [${identityColumn.name}] = ${identityFormatted}`;
      queries.push(updateQuery);
    }

    return queries;
  }, [editedCells, identityColumn, tableName, result.rows, result.columns]);

  // Show confirmation dialog with generated queries
  const handleSaveEdits = useCallback(() => {
    if (editedCells.size === 0) return;

    // Check if we can save
    if (!canSaveEdits || !onExecuteUpdate || !identityColumn) {
      setSaveError('Cannot save: Not connected to database or missing identity column.');
      return;
    }

    // Check if we have a table name
    if (!tableName) {
      setSaveError('Cannot save: Unable to detect table name from query. Use a simple SELECT FROM query.');
      return;
    }

    const queries = generateUpdateQueries();
    if (!queries || queries.length === 0) {
      setSaveError('No valid queries to execute.');
      return;
    }

    // Show confirmation dialog
    setPendingQueries(queries);
    setShowConfirmDialog(true);
    setSaveError(null);
  }, [canSaveEdits, onExecuteUpdate, identityColumn, tableName, editedCells.size, generateUpdateQueries]);

  // Execute queries after confirmation
  const handleConfirmSave = useCallback(async () => {
    if (pendingQueries.length === 0 || !onExecuteUpdate) return;

    setShowConfirmDialog(false);
    setIsSaving(true);
    setSaveError(null);

    try {
      for (let i = 0; i < pendingQueries.length; i++) {
        const success = await onExecuteUpdate(pendingQueries[i]);
        if (!success) {
          throw new Error(`Failed to execute query ${i + 1}`);
        }
      }

      // Update the grid data to reflect the saved values
      if (tabId && resultIndex !== undefined) {
        const updates = Array.from(editedCells.values()).map(edit => ({
          rowIndex: edit.rowIndex,
          colIndex: edit.colIndex,
          value: edit.newValue
        }));
        updateResultCells(tabId, resultIndex, updates);
      }

      // Clear edited cells on successful save
      setEditedCells(new Map());
      setPendingQueries([]);

    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Failed to save changes');
    } finally {
      setIsSaving(false);
    }
  }, [pendingQueries, onExecuteUpdate, tabId, resultIndex, editedCells, updateResultCells]);

  // Cancel confirmation
  const handleCancelConfirm = useCallback(() => {
    setShowConfirmDialog(false);
    setPendingQueries([]);
  }, []);

  // Click handlers - just focus container, selection is handled by mouseDown/mouseUp
  const handleCellClick = useCallback((_row: number, _col: number) => {
    // Focus the container so keyboard shortcuts work
    containerRef.current?.focus();
  }, []);

  // Mouse down handler for starting selection (also handles double-click detection)
  const handleCellMouseDown = useCallback((row: number, col: number, shiftKey: boolean) => {
    // Double-click detection
    const now = Date.now();
    const isDoubleClick =
      now - lastClickTime < DOUBLE_CLICK_DELAY &&
      lastClickRow === row &&
      lastClickCol === col;

    if (isDoubleClick && !shiftKey) {
      // Double click detected - start editing
      handleStartEdit(row, col);
      lastClickTime = 0;
      lastClickRow = -1;
      lastClickCol = -1;
      return;
    }

    // Track for potential double-click
    lastClickTime = now;
    lastClickRow = row;
    lastClickCol = col;

    if (shiftKey && selectedCell) {
      // Shift+click extends selection from current selected cell
      setSelection({
        startRow: selectedCell.row,
        startCol: selectedCell.col,
        endRow: row,
        endCol: col,
      });
    } else {
      // Start new potential selection - always set selectedCell for single-cell operations
      setSelectedCell({ row, col });
      setSelection(null); // Clear multi-selection, will be set if user drags
      setIsSelecting(true);
    }
    // Focus the container so keyboard shortcuts work
    containerRef.current?.focus();
  }, [selectedCell, handleStartEdit]);

  // Mouse enter handler for extending selection during drag
  const handleCellMouseEnter = useCallback((row: number, col: number) => {
    // Set hovered cell for preview button (negative values clear hover)
    if (row < 0 || col < 0) {
      setHoveredCell(null);
    } else {
      setHoveredCell({ row, col });
    }

    if (isSelecting && selectedCell) {
      // User is dragging - create or extend selection
      if (row !== selectedCell.row || col !== selectedCell.col) {
        setSelection({
          startRow: selectedCell.row,
          startCol: selectedCell.col,
          endRow: row,
          endCol: col,
        });
      }
    }
  }, [isSelecting, selectedCell]);

  // Mouse up handler for ending selection
  const handleCellMouseUp = useCallback(() => {
    setIsSelecting(false);
  }, []);

  // Global mouse up to handle mouse release outside grid
  useEffect(() => {
    const handleGlobalMouseUp = () => {
      setIsSelecting(false);
    };
    window.addEventListener('mouseup', handleGlobalMouseUp);
    return () => window.removeEventListener('mouseup', handleGlobalMouseUp);
  }, []);

  // Context menu handler
  const handleCellContextMenu = useCallback((e: React.MouseEvent, row: number, col: number) => {
    // If the clicked cell is not in the current selection, select only that cell
    if (!isCellInSelection(row, col, selection)) {
      setSelectedCell({ row, col });
      setSelection(null);
    }
    setContextMenu({ x: e.clientX, y: e.clientY, row, col });
  }, [selection]);

  // Row data for the virtual list
  const rowData: RowData = useMemo(() => ({
    rows: result.rows,
    columns: result.columns,
    columnWidths,
    totalWidth,
    selectedCell,
    selection,
    copiedCell,
    copiedSelection,
    editingCell,
    editedCells,
    canEdit: canActuallyEdit ?? false,
    hoveredCell,
    onCellClick: handleCellClick,
    onCellMouseDown: handleCellMouseDown,
    onCellMouseEnter: handleCellMouseEnter,
    onCellMouseUp: handleCellMouseUp,
    onCellDoubleClick: handleStartEdit,
    onCellContextMenu: handleCellContextMenu,
    columnOrder,
    onCopyRow: handleCopyRow,
    onEditChange: handleEditChange,
    onEditCommit: handleEditCommit,
    onEditCancel: handleEditCancel,
    onPreviewCell: handlePreviewCell,
  }), [result.rows, result.columns, columnWidths, totalWidth, selectedCell, selection, copiedCell, copiedSelection, editingCell?.rowIndex, editingCell?.colIndex, editedCells, canActuallyEdit, hoveredCell, handleCellClick, handleCellMouseDown, handleCellMouseEnter, handleCellMouseUp, handleStartEdit, handleCellContextMenu, columnOrder, handleCopyRow, handleEditChange, handleEditCommit, handleEditCancel, handlePreviewCell,
  ]); // Only depend on row/col, not value

  // Reordered result for export/copy
  const reorderedResult = useMemo(() => {
    // If order is default, return original
    if (columnOrder.every((idx, i) => idx === i)) return result;

    return {
      ...result,
      columns: columnOrder.map(idx => result.columns[idx]),
      // Expensive part for large results, but only computed once when needed for export/copy
      rows: result.rows.map(row => columnOrder.map(idx => row[idx]))
    };
  }, [result, columnOrder]);

  // Loading state
  if (isExecuting) {
    return (
      <div className="flex h-full">
        <div className="flex items-center justify-center h-full flex-1">
          <div className="flex items-center gap-3 text-[var(--text-secondary)]">
            <div
              className="w-5 h-5 border-2 border-t-transparent rounded-full animate-spin"
              style={{ borderColor: `${spaceColor}40`, borderTopColor: 'transparent' }}
            />
            <span>Executing query...</span>
          </div>
        </div>
      </div>
    );
  }

  // Error state
  if (result.error) {
    return (
      <div className="flex h-full">
        <div className="p-4 h-full overflow-auto flex-1">
          <div className="flex items-start gap-3 p-4 bg-red-500/10 border border-red-500/20 rounded-lg">
            <svg className="w-5 h-5 text-red-400 shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <div className="flex-1 min-w-0">
              <div className="font-medium text-red-400 mb-1">Query Error</div>
              <pre className="text-sm text-red-300/80 whitespace-pre-wrap font-mono">{result.error}</pre>
            </div>
          </div>
        </div>
      </div>
    );
  }

  // No results state
  if (result.columns.length === 0) {
    return (
      <div className="flex h-full">
        <div className="flex items-center justify-center h-full flex-1">
          <div className="text-center">
            <svg className="w-12 h-12 text-[var(--text-muted)] mx-auto mb-3 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <p className="text-[var(--text-primary)]">Query executed successfully</p>
            <p className="text-[var(--text-muted)] text-sm mt-1">
              {result.row_count} row{result.row_count !== 1 ? 's' : ''} affected • {result.execution_time_ms}ms
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full">
      {/* Main grid area */}
      <div
        ref={containerRef}
        className="flex flex-col flex-1 min-w-0 overflow-hidden outline-none"
        tabIndex={0}
        data-allow-select-all
        onFocus={() => {
          // Clear selection state when focus moves away and back
        }}
      >
        {/* Fixed header */}
        <div
          ref={headerRef}
          className="shrink-0 overflow-hidden bg-[var(--bg-tertiary)] border-b border-[var(--border-color)]"
          style={{ height: HEADER_HEIGHT }}
        >
          <DragDropContext onDragEnd={handleDragEnd}>
            <Droppable droppableId={`columns-${tabId}-${resultIndex}`} direction="horizontal">
              {(provided) => (
                <div
                  ref={provided.innerRef}
                  {...provided.droppableProps}
                  className="flex items-center"
                  style={{ width: totalWidth }}
                >
                  {/* Row number header */}
                  <div
                    className="shrink-0 w-12 px-2 h-10 flex items-center justify-center text-xs text-[var(--text-muted)] border-r border-[var(--border-color)] bg-[var(--bg-hover)]"
                    style={{ minWidth: 48 }}
                  >
                    #
                  </div>

                  {/* Column headers */}
                  {columnOrder.map((origIdx, visualIdx) => {
                    const col = result.columns[origIdx];
                    return (
                      <Draggable key={origIdx} draggableId={String(origIdx)} index={visualIdx}>
                        {(provided, snapshot) => (
                          <div
                            ref={provided.innerRef}
                            {...provided.draggableProps}
                            {...provided.dragHandleProps}
                            className={`px-3 h-10 flex border-r border-[var(--border-color)] bg-[var(--bg-hover)] group transition-all duration-200 ${hoverColumnIdx === origIdx ? 'flex-col justify-start' : 'items-center gap-2'} ${snapshot.isDragging ? 'z-50 shadow-lg !bg-[var(--bg-active)] opacity-80' : ''}`}
                            style={{
                              width: columnWidths[origIdx],
                              minWidth: columnWidths[origIdx],
                              ...provided.draggableProps.style
                            }}
                            title={`${col.name} (${col.data_type})${col.nullable ? ' - nullable' : ''}`}
                            onMouseEnter={() => setHoverColumnIdx(origIdx)}
                            onMouseLeave={() => setHoverColumnIdx(null)}
                          >
                            <span className={`font-medium text-[var(--text-primary)] transition-all duration-200 ${hoverColumnIdx === origIdx ? 'text-[10px]' : 'text-xs'}`}>
                              {col.name}
                            </span>
                            <span className={`${getTypeColor(col.data_type)} shrink-0 transition-all duration-200 ${hoverColumnIdx === origIdx ? 'text-[10px] opacity-100' : 'text-[10px] opacity-0 hidden'}`}>
                              {col.data_type}
                            </span>
                          </div>
                        )}
                      </Draggable>
                    );
                  })}
                  {provided.placeholder}
                </div>
              )}
            </Droppable>
          </DragDropContext>
        </div>

        {/* Virtual scrolling body */}
        <div ref={bodyRef} className="flex-1 min-h-0">
          {result.rows.length > 0 ? (
            <List
              key={editingCell ? `editing-${editingCell.rowIndex}-${editingCell.colIndex}` : 'no-edit'}
              defaultHeight={containerHeight}
              rowCount={result.rows.length}
              rowHeight={ROW_HEIGHT}
              rowComponent={(props) => <Row {...props} data={rowData} />}
              rowProps={{ data: rowData }}
              style={{ overflowX: 'auto', height: containerHeight }}
              className="results-grid-list"
            />
          ) : (
            <div className="flex items-center justify-center h-full text-[var(--text-muted)]">
              No rows returned
            </div>
          )}
        </div>

        {/* Status bar */}
        <div className="shrink-0 flex items-center justify-between px-4 py-2 bg-[var(--bg-tertiary)] border-t border-[var(--border-color)] text-xs text-[var(--text-muted)]">
          <div className="flex items-center gap-4">
            <span>
              <strong className="text-[var(--text-primary)]">{result.row_count.toLocaleString()}</strong> row{result.row_count !== 1 ? 's' : ''}
            </span>
            <span>
              <strong className="text-[var(--text-primary)]">{result.columns.length}</strong> column{result.columns.length !== 1 ? 's' : ''}
            </span>
            <span>
              <strong className="text-[var(--text-primary)]">{result.execution_time_ms}</strong>ms
            </span>
            {/* Edit status indicator */}
            {editedCells.size > 0 && (
              <span className="px-2 py-0.5 bg-yellow-500/20 text-yellow-400 rounded">
                {editedCells.size} pending edit{editedCells.size !== 1 ? 's' : ''}
              </span>
            )}
            {saveError && (
              <span className="px-2 py-0.5 bg-red-500/20 text-red-400 rounded" title={saveError}>
                Save failed
              </span>
            )}
          </div>

          <div className="flex items-center gap-2">
            {selectedCell && (
              <span className="px-2 py-0.5 bg-[var(--bg-hover)] rounded">
                Row {selectedCell.row + 1}, Col {selectedCell.col + 1}
              </span>
            )}

            {/* Edit action buttons */}
            {editedCells.size > 0 && (
              <>
                <button
                  onClick={handleDiscardEdits}
                  disabled={isSaving}
                  className="px-2 py-1 rounded bg-[var(--bg-hover)] hover:bg-red-500/20 hover:text-red-400 transition-colors disabled:opacity-50"
                  title="Discard all changes"
                >
                  Discard
                </button>
                <button
                  onClick={handleSaveEdits}
                  disabled={isSaving || !canSaveEdits}
                  className="px-3 py-1 rounded text-white transition-colors disabled:opacity-50 flex items-center gap-1"
                  style={{ backgroundColor: canSaveEdits ? spaceColor : '#666' }}
                  title={canSaveEdits ? "Save all changes to database" : "Cannot save: Not connected to database"}
                >
                  {isSaving ? (
                    <>
                      <div className="w-3 h-3 border border-white/40 border-t-white rounded-full animate-spin" />
                      Saving...
                    </>
                  ) : (
                    <>
                      <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                      </svg>
                      Save
                    </>
                  )}
                </button>
              </>
            )}

            {/* Editing hint */}
            {editedCells.size === 0 && (
              <span className="text-[var(--text-muted)] text-[10px]">
                Double-click to edit
              </span>
            )}

            <ExportButton result={reorderedResult} spaceColor={spaceColor} />
            <button
              onClick={onClose}
              className="px-2 py-1 rounded hover:bg-[var(--bg-active)] transition-colors"
            >
              Close
            </button>
          </div>
        </div>

        {/* Context menu */}
        {contextMenu && (
          <ContextMenu
            position={{ x: contextMenu.x, y: contextMenu.y }}
            onClose={() => setContextMenu(null)}
            items={[
              ...(!selection && contextMenu ? [
                {
                  id: 'copy-cell-headers',
                  label: 'Copy Cell with Header',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
                    </svg>
                  ),
                  shortcut: 'Ctrl+Shift+C',
                  action: () => handleCopyCell(contextMenu.row, contextMenu.col, true),
                },
                {
                  id: 'copy-cell',
                  label: 'Copy Cell',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                    </svg>
                  ),
                  shortcut: 'Ctrl+C',
                  action: () => handleCopyCell(contextMenu.row, contextMenu.col),
                }
              ] : []),
              ...(selection ? [
                {
                  id: 'copy-selection-headers',
                  label: 'Copy Selection with Headers',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
                    </svg>
                  ),
                  shortcut: 'Ctrl+Shift+C',
                  action: () => handleCopySelection(selection, true),
                },
                {
                  id: 'copy-selection',
                  label: 'Copy Selection',
                  icon: (
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 10h16M4 14h16M4 18h16" />
                    </svg>
                  ),
                  shortcut: 'Ctrl+C',
                  action: () => handleCopySelection(selection),
                }
              ] : []),
              {
                id: 'copy-as-insert',
                label: selection ? 'Copy as INSERT' : 'Copy Row as INSERT',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                  </svg>
                ),
                action: () => {
                  if (selection) {
                    handleCopyAsInsert(selection);
                  } else if (contextMenu) {
                    handleCopyAsInsert({
                      startRow: contextMenu.row,
                      startCol: 0,
                      endRow: contextMenu.row,
                      endCol: result.columns.length - 1,
                    });
                  }
                },
              },
              ...(selection ? [{
                id: 'copy-as-in-clause',
                label: 'Copy as IN CLAUSE',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
                  </svg>
                ),
                action: () => handleCopyAsInClause(selection),
              }] : []),
              ...(contextMenu ? [{
                id: 'copy-row',
                label: 'Copy Row (JSON)',
                icon: (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                  </svg>
                ),
                action: () => handleCopyRow(contextMenu.row),
              }] : []),
            ]}
          />
        )}

        {/* Confirmation Dialog for Save */}
        {showConfirmDialog && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
            <div className="bg-[var(--bg-primary)] border border-[var(--border-color)] rounded-lg shadow-2xl max-w-2xl w-full mx-4 max-h-[80vh] flex flex-col">
              {/* Header */}
              <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--border-color)]">
                <div className="flex items-center gap-2">
                  <svg className="w-5 h-5 text-yellow-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                  </svg>
                  <h3 className="font-medium text-[var(--text-primary)]">Confirm Changes</h3>
                </div>
                <button
                  onClick={handleCancelConfirm}
                  className="p-1 rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors"
                >
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>

              {/* Content */}
              <div className="flex-1 overflow-auto p-4">
                <p className="text-sm text-[var(--text-secondary)] mb-3">
                  The following {pendingQueries.length} UPDATE {pendingQueries.length === 1 ? 'query' : 'queries'} will be executed:
                </p>
                <div className="space-y-2">
                  {pendingQueries.map((query, index) => (
                    <div key={index} className="p-3 bg-[var(--bg-secondary)] border border-[var(--border-color)] rounded-lg">
                      <pre className="text-xs font-mono text-[var(--text-primary)] whitespace-pre-wrap break-all">
                        {query}
                      </pre>
                    </div>
                  ))}
                </div>
              </div>

              {/* Footer */}
              <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-[var(--border-color)]">
                <button
                  onClick={handleCancelConfirm}
                  className="px-4 py-2 text-sm rounded-lg bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleConfirmSave}
                  className="px-4 py-2 text-sm rounded-lg bg-green-600 hover:bg-green-500 text-white font-medium transition-colors"
                >
                  Execute {pendingQueries.length} {pendingQueries.length === 1 ? 'Query' : 'Queries'}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Cell preview panel */}
      {cellPreviewPanel.visible && (
        <CellPreviewPanel
          visible={cellPreviewPanel.visible}
          width={cellPreviewPanel.width}
          selectedCell={cellPreviewPanel.selectedCell}
          formatterType={cellPreviewPanel.formatterType}
          onClose={hideCellPreview}
          onResize={setCellPreviewWidth}
          onResizeImmediate={setCellPreviewWidthImmediate}
          onFormatChange={setCellPreviewFormatter}
        />
      )}
    </div>
  );
}

export const ResultsGrid = memo(ResultsGridComp);
