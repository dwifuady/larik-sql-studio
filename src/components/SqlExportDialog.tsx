
import { useState } from 'react';
import { createPortal } from 'react-dom';

interface SqlExportDialogProps {
    isOpen: boolean;
    onClose: () => void;
    onConfirm: (includeNotes: boolean) => void;
    hasNotes: boolean;
}

export function SqlExportDialog({ isOpen, onClose, onConfirm, hasNotes }: SqlExportDialogProps) {
    const [includeNotes, setIncludeNotes] = useState(true);

    if (!isOpen) return null;

    return createPortal(
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
            <div className="rounded-lg shadow-xl border w-full max-w-md p-6 transform transition-all scale-100"
                style={{ backgroundColor: 'var(--bg-secondary)', borderColor: 'var(--border-color)' }}>
                <h3 className="text-lg font-semibold mb-2" style={{ color: 'var(--text-primary)' }}>
                    Export SQL
                </h3>

                <p className="mb-6 text-sm" style={{ color: 'var(--text-muted)' }}>
                    {hasNotes
                        ? "This query contains Sticky Notes. How would you like to handle them?"
                        : "Export this query to a SQL file?"}
                </p>

                {hasNotes && (
                    <div className="mb-6 space-y-3">
                        <label className="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors"
                            style={{ borderColor: 'var(--border-color)' }}
                            onMouseEnter={(e) => e.currentTarget.style.backgroundColor = 'var(--bg-hover)'}
                            onMouseLeave={(e) => e.currentTarget.style.backgroundColor = 'transparent'}>
                            <input
                                type="radio"
                                name="export-notes"
                                className="mt-1"
                                checked={includeNotes}
                                onChange={() => setIncludeNotes(true)}
                            />
                            <div className="flex-1">
                                <span className="block text-sm font-medium" style={{ color: 'var(--text-primary)' }}>
                                    Include Sticky Notes
                                </span>
                                <span className="block text-xs" style={{ color: 'var(--text-muted)' }}>
                                    Notes will be saved as comments (`-- @note: ...`)
                                </span>
                            </div>
                        </label>

                        <label className="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors"
                            style={{ borderColor: 'var(--border-color)' }}
                            onMouseEnter={(e) => e.currentTarget.style.backgroundColor = 'var(--bg-hover)'}
                            onMouseLeave={(e) => e.currentTarget.style.backgroundColor = 'transparent'}>
                            <input
                                type="radio"
                                name="export-notes"
                                className="mt-1"
                                checked={!includeNotes}
                                onChange={() => setIncludeNotes(false)}
                            />
                            <div className="flex-1">
                                <span className="block text-sm font-medium" style={{ color: 'var(--text-primary)' }}>
                                    Exclude Sticky Notes
                                </span>
                                <span className="block text-xs" style={{ color: 'var(--text-muted)' }}>
                                    Export only raw SQL code
                                </span>
                            </div>
                        </label>
                    </div>
                )}

                <div className="flex justify-end gap-3">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 text-sm font-medium transition-colors"
                        style={{ color: 'var(--text-muted)' }}
                    >
                        Cancel
                    </button>
                    <button
                        onClick={() => {
                            onConfirm(hasNotes ? includeNotes : true);
                            onClose();
                        }}
                        className="px-4 py-2 text-sm font-medium text-white rounded-md shadow-sm transition-colors"
                        style={{ backgroundColor: 'var(--accent-color)' }}
                    >
                        Export
                    </button>
                </div>
            </div>
        </div>,
        document.body
    );
}
