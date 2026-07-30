import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, Link2, Search, X } from 'lucide-react';
import { useAppStore } from '../store';
import type { SchemaColumnInfo, TableInfo } from '../types';

/**
 * Define a user-defined ("virtual") reference for a column whose lookup table
 * isn't declared as a foreign key in the database. Stored locally by Larik —
 * nothing is written to the server.
 */
export function SetReferenceDialog() {
    const editor = useAppStore((s) => s.referenceEditor);
    const closeEditor = useAppStore((s) => s.closeReferenceEditor);
    const saveReference = useAppStore((s) => s.saveVirtualReference);
    const schemaInfo = useAppStore((s) => s.schemaInfo);

    const [tableFilter, setTableFilter] = useState('');
    const [selectedTable, setSelectedTable] = useState<TableInfo | null>(null);
    const [selectedColumn, setSelectedColumn] = useState<string | null>(null);
    const dialogRef = useRef<HTMLDivElement>(null);

    const tables = useMemo(() => schemaInfo?.tables ?? [], [schemaInfo]);

    // Prefill from the reference being edited each time the dialog opens.
    useEffect(() => {
        if (!editor.open) return;

        setTableFilter('');
        const existing = editor.existingTarget;
        if (!existing) {
            setSelectedTable(null);
            setSelectedColumn(null);
            return;
        }
        const table = tables.find(
            (candidate) =>
                candidate.schema_name.toLowerCase() === existing.schema.toLowerCase() &&
                candidate.table_name.toLowerCase() === existing.table.toLowerCase()
        );
        setSelectedTable(table ?? null);
        setSelectedColumn(existing.column);
    }, [editor.open, editor.existingTarget, tables]);

    // Close on Escape
    useEffect(() => {
        if (!editor.open) return;
        const onKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                closeEditor();
            }
        };
        document.addEventListener('keydown', onKeyDown);
        return () => document.removeEventListener('keydown', onKeyDown);
    }, [editor.open, closeEditor]);

    const filteredTables = useMemo(() => {
        const term = tableFilter.trim().toLowerCase();
        const base = tables.filter((table) => table.table_type === 'BASE TABLE' || table.table_type === 'VIEW');
        if (!term) return base.slice(0, 300);
        return base
            .filter((table) => `${table.schema_name}.${table.table_name}`.toLowerCase().includes(term))
            .slice(0, 300);
    }, [tables, tableFilter]);

    /** Default to the target table's primary key — the usual lookup key. */
    const suggestedColumn = (table: TableInfo): string | null => {
        const pk = table.columns.find((column) => column.is_primary_key);
        return pk?.name ?? table.columns[0]?.name ?? null;
    };

    if (!editor.open || !editor.source) return null;

    const source = editor.source;
    const canSave = Boolean(selectedTable && selectedColumn);

    const handleSave = async () => {
        if (!selectedTable || !selectedColumn) return;
        await saveReference({
            sourceSchema: source.schema,
            sourceTable: source.table,
            sourceColumn: source.column,
            targetSchema: selectedTable.schema_name,
            targetTable: selectedTable.table_name,
            targetColumn: selectedColumn,
        });
    };

    const columnLabel = (column: SchemaColumnInfo) =>
        `${column.name}${column.is_primary_key ? ' · PK' : ''} · ${column.data_type}`;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
            <div
                ref={dialogRef}
                className="flex flex-col w-[560px] max-w-[92vw] max-h-[80vh] bg-[var(--bg-primary)] border border-[var(--border-color)] rounded-xl shadow-2xl overflow-hidden"
            >
                {/* Header */}
                <div className="shrink-0 flex items-start justify-between gap-3 px-4 py-3 border-b border-[var(--border-color)]">
                    <div className="min-w-0">
                        <div className="flex items-center gap-2">
                            <Link2 className="w-4 h-4 text-[var(--warning-color)]" />
                            <h3 className="text-sm font-medium text-[var(--text-primary)]">
                                {editor.existingId ? 'Edit custom reference' : 'Set custom reference'}
                            </h3>
                        </div>
                        <p className="mt-1 text-xs text-[var(--text-muted)]">
                            <span className="font-mono text-[var(--text-secondary)]">
                                {source.schema}.{source.table}.{source.column}
                            </span>{' '}
                            → pick the table it looks up. Saved in Larik only, never on the server.
                        </p>
                    </div>
                    <button
                        onClick={closeEditor}
                        className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
                        title="Close (Esc)"
                    >
                        <X className="w-4 h-4" />
                    </button>
                </div>

                {/* Body: table picker then column picker */}
                <div className="flex-1 min-h-0 flex">
                    <div className="flex flex-col w-1/2 min-w-0 border-r border-[var(--border-color)]">
                        <div className="shrink-0 p-2">
                            <div className="relative">
                                <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-[var(--text-muted)]" />
                                <input
                                    autoFocus
                                    value={tableFilter}
                                    onChange={(e) => setTableFilter(e.target.value)}
                                    placeholder="Find reference table"
                                    className="w-full pl-7 pr-2 py-1.5 text-xs rounded-md bg-[var(--bg-tertiary)] border border-[var(--border-color)] text-[var(--text-primary)] placeholder:text-[var(--text-muted)] outline-none focus:border-[var(--accent-color)] transition-colors"
                                />
                            </div>
                        </div>
                        <div className="flex-1 min-h-0 overflow-auto px-1 pb-2">
                            {filteredTables.map((table) => {
                                const isSelected =
                                    selectedTable?.schema_name === table.schema_name &&
                                    selectedTable?.table_name === table.table_name;
                                return (
                                    <button
                                        key={`${table.schema_name}.${table.table_name}`}
                                        onClick={() => {
                                            setSelectedTable(table);
                                            setSelectedColumn(suggestedColumn(table));
                                        }}
                                        className={`w-full px-2 py-1.5 text-left text-xs rounded-md truncate transition-colors ${
                                            isSelected
                                                ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                                                : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                                        }`}
                                        title={`${table.schema_name}.${table.table_name}`}
                                    >
                                        <span className="text-[var(--text-muted)]">{table.schema_name}.</span>
                                        {table.table_name}
                                    </button>
                                );
                            })}
                            {filteredTables.length === 0 && (
                                <div className="px-2 py-6 text-center text-xs text-[var(--text-muted)]">
                                    No tables match "{tableFilter}".
                                </div>
                            )}
                        </div>
                    </div>

                    <div className="flex flex-col w-1/2 min-w-0">
                        <div className="shrink-0 px-3 py-2 text-[10px] uppercase tracking-wide text-[var(--text-muted)] border-b border-[var(--border-color)]">
                            {selectedTable ? `Key column in ${selectedTable.table_name}` : 'Key column'}
                        </div>
                        <div className="flex-1 min-h-0 overflow-auto px-1 py-1">
                            {selectedTable ? (
                                selectedTable.columns.map((column) => (
                                    <button
                                        key={column.name}
                                        onClick={() => setSelectedColumn(column.name)}
                                        className={`w-full px-2 py-1.5 text-left text-xs rounded-md truncate transition-colors ${
                                            selectedColumn === column.name
                                                ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                                                : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                                        }`}
                                        title={columnLabel(column)}
                                    >
                                        {column.name}
                                        <span className="ml-1 text-[10px] text-[var(--text-muted)]">
                                            {column.is_primary_key ? 'PK · ' : ''}
                                            {column.data_type}
                                        </span>
                                    </button>
                                ))
                            ) : (
                                <div className="px-2 py-6 text-center text-xs text-[var(--text-muted)]">
                                    Pick a table first.
                                </div>
                            )}
                        </div>
                    </div>
                </div>

                {/* Footer */}
                <div className="shrink-0 flex items-center justify-between gap-2 px-4 py-3 border-t border-[var(--border-color)]">
                    <div className="text-xs text-[var(--text-muted)] truncate">
                        {canSave ? (
                            <span className="font-mono">
                                {source.column} → {selectedTable!.schema_name}.{selectedTable!.table_name}.
                                {selectedColumn}
                            </span>
                        ) : (
                            'Select the table and key column this value points to.'
                        )}
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                        <button
                            onClick={closeEditor}
                            className="px-3 py-1.5 text-xs rounded-lg bg-[var(--bg-hover)] hover:bg-[var(--bg-active)] text-[var(--text-primary)] transition-colors"
                        >
                            Cancel
                        </button>
                        <button
                            onClick={() => void handleSave()}
                            disabled={!canSave || editor.saving}
                            className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-lg bg-[var(--accent-color)] hover:bg-[var(--accent-hover)] text-white font-medium transition-colors disabled:opacity-50"
                        >
                            <Check className="w-3.5 h-3.5" />
                            {editor.saving ? 'Saving…' : 'Save reference'}
                        </button>
                    </div>
                </div>
            </div>
        </div>
    );
}
