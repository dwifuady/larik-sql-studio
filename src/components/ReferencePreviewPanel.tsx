import { useEffect, useMemo, useRef, useState } from 'react';
import { ExternalLink, RefreshCw, Search, TriangleAlert } from 'lucide-react';
import { useAppStore } from '../store';
import type { CellValue } from '../types';

/** Plain-text rendering of a cell for the compact reference table. */
function renderValue(value: CellValue): string {
    if (value === null || value === undefined) return 'NULL';
    if (typeof value === 'boolean') return value ? 'true' : 'false';
    if (Array.isArray(value)) return '[binary]';
    const text = String(value);
    if (text.length === 0) return '(empty)';
    return text.length > 300 ? `${text.slice(0, 300)}…` : text;
}

function isNullValue(value: CellValue): boolean {
    return value === null || value === undefined;
}

/**
 * Reference data view for the cell preview panel: shows the referenced
 * (dictionary) table for a foreign key column and highlights the row the
 * current cell points at.
 *
 * Big tables are never listed in full — see referencePreviewSlice for the
 * row-estimate / lookup-mode logic.
 */
export function ReferencePreviewPanel() {
    const preview = useAppStore((s) => s.referencePreview);
    const reload = useAppStore((s) => s.reloadReferencePreview);
    const loadListAnyway = useAppStore((s) => s.loadReferenceListAnyway);
    const openInTab = useAppStore((s) => s.openReferenceInTab);
    const rowLimit = useAppStore((s) => s.referencePreviewRowLimit);

    const [filterText, setFilterText] = useState('');
    const matchedRowRef = useRef<HTMLTableRowElement | null>(null);

    const { result, matchedRowIndex, mode, target, loading, error } = preview;

    // Reset the filter whenever a different reference is loaded.
    useEffect(() => {
        setFilterText('');
    }, [preview.requestKey]);

    // Bring the highlighted row into view once rows arrive.
    useEffect(() => {
        if (matchedRowIndex === null) return;
        matchedRowRef.current?.scrollIntoView({ block: 'center' });
    }, [matchedRowIndex, result, filterText]);

    const visibleRows = useMemo(() => {
        if (!result) return [] as Array<{ row: CellValue[]; index: number }>;
        const all = result.rows.map((row, index) => ({ row, index }));
        const term = filterText.trim().toLowerCase();
        if (!term) return all;
        return all.filter(({ row }) =>
            row.some((cell) => !isNullValue(cell) && String(cell).toLowerCase().includes(term))
        );
    }, [result, filterText]);

    if (!target) {
        return (
            <div className="flex items-center justify-center h-full px-6 text-center text-sm text-[var(--text-muted)]">
                Select a cell in a foreign key column to see its reference data.
            </div>
        );
    }

    const targetLabel = `${target.schema}.${target.table}`;
    const matchedRow = matchedRowIndex !== null ? result?.rows[matchedRowIndex] ?? null : null;

    return (
        <div className="flex flex-col h-full min-h-0">
            {/* Reference header */}
            <div className="shrink-0 px-3 py-2 border-b border-[var(--border-color)] bg-[var(--bg-secondary)]">
                <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                        <div className="flex items-center gap-1.5 min-w-0">
                            <span className="text-[10px] uppercase tracking-wide text-[var(--text-muted)] shrink-0">
                                references
                            </span>
                            <span className="text-[12px] font-medium text-[var(--text-primary)] truncate" title={targetLabel}>
                                {targetLabel}
                            </span>
                        </div>
                        <div className="mt-0.5 text-[10px] text-[var(--text-muted)] truncate" title={preview.constraintName ?? undefined}>
                            {preview.keyColumn}
                            {preview.estimatedRows !== null && (
                                <> · ~{preview.estimatedRows.toLocaleString()} rows</>
                            )}
                            {mode === 'lookup' && <> · matching row only</>}
                            {mode === 'list' && <> · full list</>}
                        </div>
                    </div>

                    <div className="flex items-center gap-1 shrink-0">
                        <button
                            onClick={() => void reload()}
                            disabled={loading}
                            className="p-1.5 rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors disabled:opacity-50"
                            title="Reload reference data"
                        >
                            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
                        </button>
                        <button
                            onClick={() => void openInTab()}
                            className="p-1.5 rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors"
                            title="Open reference table in a new tab"
                        >
                            <ExternalLink className="w-3.5 h-3.5" />
                        </button>
                    </div>
                </div>

                {preview.skippedColumns.length > 0 && (
                    <div className="mt-2 flex items-start gap-1.5 text-[10px] text-[var(--warning-color)]">
                        <TriangleAlert className="w-3 h-3 mt-px shrink-0" />
                        <span>
                            Composite key: could not filter on {preview.skippedColumns.join(', ')} (not in this result set).
                        </span>
                    </div>
                )}
            </div>

            {/* Body */}
            {loading && !result ? (
                <div className="flex items-center justify-center flex-1 gap-2 text-sm text-[var(--text-secondary)]">
                    <RefreshCw className="w-4 h-4 animate-spin" />
                    <span>Loading reference data…</span>
                </div>
            ) : error ? (
                <div className="p-3 overflow-auto">
                    <div className="p-3 rounded-lg bg-red-500/10 border border-red-500/20">
                        <div className="text-[12px] font-medium text-red-400 mb-1">Reference lookup failed</div>
                        <pre className="text-[11px] text-red-300/80 whitespace-pre-wrap font-mono">{error}</pre>
                    </div>
                </div>
            ) : preview.hasNullValue && !(result && result.rows.length > 0) ? (
                <div className="flex flex-col items-center justify-center flex-1 gap-3 px-6 text-center">
                    <div className="text-2xl text-[var(--text-muted)]">∅</div>
                    <div className="text-sm text-[var(--text-secondary)]">
                        This cell is NULL — no referenced row.
                    </div>
                    {mode !== 'list' && (
                        <button
                            onClick={() => void loadListAnyway()}
                            className="px-3 py-1.5 text-[11px] rounded-md bg-[var(--bg-tertiary)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
                        >
                            Show first {rowLimit} rows of {target.table}
                        </button>
                    )}
                </div>
            ) : mode === 'lookup' && (!result || result.rows.length === 0) ? (
                <div className="flex flex-col items-center justify-center flex-1 gap-3 px-6 text-center">
                    <div className="text-sm text-[var(--text-secondary)]">
                        No row in <span className="font-medium text-[var(--text-primary)]">{target.table}</span> matches{' '}
                        <span className="font-mono text-[var(--text-primary)]">{renderValue(preview.value)}</span>.
                    </div>
                    <div className="text-[11px] text-[var(--text-muted)]">
                        The table is larger than the {rowLimit.toLocaleString()}-row preview limit, so only the matching
                        row is fetched.
                    </div>
                    <button
                        onClick={() => void loadListAnyway()}
                        className="px-3 py-1.5 text-[11px] rounded-md bg-[var(--bg-tertiary)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
                    >
                        Load first {rowLimit.toLocaleString()} rows anyway
                    </button>
                </div>
            ) : mode === 'lookup' && result && result.rows.length === 1 && matchedRow ? (
                /* Single matched row reads better as a record card in a narrow pane. */
                <div className="flex-1 min-h-0 flex flex-col">
                    <div className="flex-1 overflow-auto p-3">
                        <div className="rounded-lg border border-[var(--accent-color)]/40 bg-[var(--accent-glow)] divide-y divide-[var(--border-color)]">
                            {result.columns.map((column, index) => (
                                <div key={column.name} className="flex gap-3 px-3 py-1.5">
                                    <span
                                        className="w-32 shrink-0 text-[11px] text-[var(--text-secondary)] truncate"
                                        title={`${column.name} (${column.data_type})`}
                                    >
                                        {column.name}
                                    </span>
                                    <span
                                        className={`text-[11px] font-mono break-all ${
                                            isNullValue(matchedRow[index])
                                                ? 'text-[var(--text-muted)] italic'
                                                : 'text-[var(--text-primary)]'
                                        }`}
                                    >
                                        {renderValue(matchedRow[index])}
                                    </span>
                                </div>
                            ))}
                        </div>
                    </div>
                    <div className="shrink-0 flex items-center justify-between gap-2 px-3 py-2 border-t border-[var(--border-color)] text-[10px] text-[var(--text-muted)]">
                        <span>Matched row</span>
                        <button
                            onClick={() => void loadListAnyway()}
                            className="px-2 py-1 rounded bg-[var(--bg-tertiary)] hover:bg-[var(--bg-active)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] transition-colors"
                        >
                            Load first {rowLimit.toLocaleString()} rows
                        </button>
                    </div>
                </div>
            ) : result ? (
                <div className="flex-1 min-h-0 flex flex-col">
                    {/* Client-side filter over the already-fetched rows */}
                    <div className="shrink-0 px-3 py-2 border-b border-[var(--border-color)]">
                        <div className="relative">
                            <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-[var(--text-muted)]" />
                            <input
                                value={filterText}
                                onChange={(e) => setFilterText(e.target.value)}
                                placeholder="Filter loaded rows"
                                className="w-full pl-7 pr-2 py-1 text-[11px] rounded-md bg-[var(--bg-tertiary)] border border-[var(--border-color)] text-[var(--text-primary)] placeholder:text-[var(--text-muted)] outline-none focus:border-[var(--accent-color)] transition-colors"
                            />
                        </div>
                    </div>

                    <div className="flex-1 min-h-0 overflow-auto">
                        <table className="w-full text-[11px] border-collapse">
                            <thead className="sticky top-0 z-10">
                                <tr className="bg-[var(--bg-tertiary)]">
                                    {result.columns.map((column) => (
                                        <th
                                            key={column.name}
                                            className="px-2 py-1.5 text-left font-medium text-[var(--text-secondary)] border-b border-[var(--border-color)] whitespace-nowrap"
                                            title={`${column.name} (${column.data_type})`}
                                        >
                                            {column.name}
                                        </th>
                                    ))}
                                </tr>
                            </thead>
                            <tbody>
                                {visibleRows.map(({ row, index }) => {
                                    const isMatch = index === matchedRowIndex;
                                    return (
                                        <tr
                                            key={index}
                                            ref={isMatch ? matchedRowRef : undefined}
                                            className={
                                                isMatch
                                                    ? 'bg-[var(--accent-glow)] outline outline-1 -outline-offset-1 outline-[var(--accent-color)]/50'
                                                    : 'hover:bg-[var(--bg-hover)]'
                                            }
                                        >
                                            {row.map((cell, cellIndex) => (
                                                <td
                                                    key={cellIndex}
                                                    className={`px-2 py-1 border-b border-[var(--border-subtle)] font-mono whitespace-nowrap max-w-[280px] truncate ${
                                                        isNullValue(cell)
                                                            ? 'text-[var(--text-muted)] italic'
                                                            : isMatch
                                                                ? 'text-[var(--text-primary)] font-semibold'
                                                                : 'text-[var(--text-primary)]'
                                                    }`}
                                                    title={renderValue(cell)}
                                                >
                                                    {renderValue(cell)}
                                                </td>
                                            ))}
                                        </tr>
                                    );
                                })}
                            </tbody>
                        </table>

                        {visibleRows.length === 0 && (
                            <div className="px-3 py-6 text-center text-[11px] text-[var(--text-muted)]">
                                No loaded rows match "{filterText}".
                            </div>
                        )}
                    </div>

                    {/* Footer status */}
                    <div className="shrink-0 flex items-center justify-between gap-2 px-3 py-2 border-t border-[var(--border-color)] text-[10px] text-[var(--text-muted)]">
                        <span>
                            {filterText.trim()
                                ? `${visibleRows.length.toLocaleString()} of ${result.rows.length.toLocaleString()} rows`
                                : `${result.rows.length.toLocaleString()} row${result.rows.length === 1 ? '' : 's'}`}
                            {matchedRowIndex !== null && <> · match on row {matchedRowIndex + 1}</>}
                        </span>
                        {matchedRowIndex === null && !preview.hasNullValue && (
                            <span className="px-1.5 py-0.5 rounded bg-amber-500/20 text-amber-300">
                                {preview.truncated ? 'match not in loaded rows' : 'no match'}
                            </span>
                        )}
                    </div>
                </div>
            ) : (
                <div className="flex items-center justify-center flex-1 text-sm text-[var(--text-muted)]">
                    No reference data.
                </div>
            )}
        </div>
    );
}
