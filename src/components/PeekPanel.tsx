import { useEffect } from 'react';
import { Table, Eye, Binary, X, Maximize2, RefreshCw } from 'lucide-react';
import { useAppStore } from '../store';
import { ResultsGrid } from './ResultsGrid';
import { MonacoPreview } from './MonacoPreview';

/**
 * Arc-style "peek" popup for the Database Explorer.
 * - Shift+click a table/view shows a TOP 100 data preview (ResultsGrid).
 * - Alt+click a view/procedure shows its source (read-only Monaco editor).
 * The Expand button converts the peek into a full query tab.
 */
export function PeekPanel() {
    const peek = useAppStore((s) => s.peek);
    const closePeek = useAppStore((s) => s.closePeek);
    const expandPeek = useAppStore((s) => s.expandPeek);
    const spaces = useAppStore((s) => s.spaces);
    const activeSpaceId = useAppStore((s) => s.activeSpaceId);

    const spaceColor = spaces.find((s) => s.id === activeSpaceId)?.color || '#6366f1';

    // Close on Escape.
    useEffect(() => {
        if (!peek.open) return;
        const onKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                closePeek();
            }
        };
        window.addEventListener('keydown', onKeyDown);
        return () => window.removeEventListener('keydown', onKeyDown);
    }, [peek.open, closePeek]);

    if (!peek.open) return null;

    const Icon = peek.objectType === 'view' ? Eye : peek.objectType === 'routine' ? Binary : Table;
    const typeLabel =
        peek.objectType === 'view' ? 'View' : peek.objectType === 'routine' ? 'Procedure' : 'Table';

    return (
        <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px]"
            onClick={closePeek}
        >
            <div
                className="flex flex-col w-[78vw] h-[76vh] max-w-[1200px] bg-[var(--bg-primary)] border border-[var(--border-color)] rounded-xl shadow-2xl overflow-hidden"
                onClick={(e) => e.stopPropagation()}
                style={{ boxShadow: `0 20px 60px rgba(0,0,0,0.45), 0 0 0 1px ${spaceColor}22` }}
            >
                {/* Header */}
                <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--border-color)] bg-[var(--bg-secondary)] shrink-0">
                    <div className="flex items-center gap-2 min-w-0">
                        <div
                            className="w-6 h-6 rounded-md flex items-center justify-center shrink-0"
                            style={{ backgroundColor: `${spaceColor}20`, color: spaceColor }}
                        >
                            <Icon className="w-3.5 h-3.5" />
                        </div>
                        <div className="flex items-baseline gap-2 min-w-0">
                            <span className="text-sm font-medium text-[var(--text-primary)] truncate">
                                {peek.schema}.{peek.name}
                            </span>
                            <span className="text-[10px] uppercase tracking-wide text-[var(--text-muted)] shrink-0">
                                {typeLabel} peek
                            </span>
                        </div>
                    </div>

                    <div className="flex items-center gap-1 shrink-0">
                        <button
                            onClick={expandPeek}
                            className="flex items-center gap-1 px-2 py-1 text-xs rounded-md text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
                            title="Open in a new tab"
                        >
                            <Maximize2 className="w-3.5 h-3.5" />
                            Expand
                        </button>
                        <button
                            onClick={closePeek}
                            className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
                            title="Close (Esc)"
                        >
                            <X className="w-4 h-4" />
                        </button>
                    </div>
                </div>

                {/* Body */}
                <div className="flex-1 min-h-0 overflow-hidden">
                    {peek.loading ? (
                        <div className="flex items-center justify-center h-full text-[var(--text-secondary)] gap-2">
                            <RefreshCw className="w-4 h-4 animate-spin" />
                            <span>Loading…</span>
                        </div>
                    ) : peek.error ? (
                        <div className="p-4 h-full overflow-auto">
                            <div className="flex items-start gap-3 p-4 bg-red-500/10 border border-red-500/20 rounded-lg">
                                <svg className="w-5 h-5 text-red-400 shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                                </svg>
                                <div className="flex-1 min-w-0">
                                    <div className="font-medium text-red-400 mb-1">Peek failed</div>
                                    <pre className="text-sm text-red-300/80 whitespace-pre-wrap font-mono">{peek.error}</pre>
                                </div>
                            </div>
                        </div>
                    ) : peek.kind === 'table-data' && peek.result ? (
                        <ResultsGrid
                            result={peek.result}
                            onClose={closePeek}
                            spaceColor={spaceColor}
                            canEdit={false}
                            queryText={peek.query ?? undefined}
                        />
                    ) : peek.kind === 'object-source' && peek.content !== null ? (
                        <MonacoPreview content={peek.content} language="sql" />
                    ) : null}
                </div>
            </div>
        </div>
    );
}
