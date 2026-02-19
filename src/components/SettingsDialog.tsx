import { useEffect, useRef } from 'react';
import { useAppStore } from '../store';

interface SettingsDialogProps {
    isOpen: boolean;
    onClose: () => void;
}

export function SettingsDialog({ isOpen, onClose }: SettingsDialogProps) {
    const dialogRef = useRef<HTMLDivElement>(null);

    const {
        enableStickyNotes,
        setEnableStickyNotes,
        maxResultRows,
        setMaxResultRows,
        validationEnabled,
        setValidationEnabled,
        validationShowWarnings,
        setValidationShowWarnings,
        validationShowInfo,
        setValidationShowInfo,
        autoArchiveSettings,
        updateAutoArchiveSettings,
    } = useAppStore();

    // Close on Escape
    useEffect(() => {
        if (!isOpen) return;

        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                onClose();
            }
        };

        document.addEventListener('keydown', handleKeyDown);
        return () => document.removeEventListener('keydown', handleKeyDown);
    }, [isOpen, onClose]);

    // Close on click outside
    useEffect(() => {
        if (!isOpen) return;

        const handleClickOutside = (e: MouseEvent) => {
            if (dialogRef.current && !dialogRef.current.contains(e.target as Node)) {
                onClose();
            }
        };

        // Delay to avoid immediate close from the trigger click
        const timer = setTimeout(() => {
            document.addEventListener('mousedown', handleClickOutside);
        }, 0);

        return () => {
            clearTimeout(timer);
            document.removeEventListener('mousedown', handleClickOutside);
        };
    }, [isOpen, onClose]);

    // Focus trap
    useEffect(() => {
        if (isOpen && dialogRef.current) {
            dialogRef.current.focus();
        }
    }, [isOpen]);

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm animate-in fade-in duration-150">
            <div
                ref={dialogRef}
                tabIndex={-1}
                className="w-full max-w-lg overflow-hidden rounded-2xl border border-white/10 bg-[var(--bg-secondary)] shadow-2xl animate-in zoom-in-95 duration-200"
            >
                {/* Header */}
                <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-white/5">
                            <svg className="w-5 h-5 text-[var(--accent-color)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                            </svg>
                        </div>
                        <div>
                            <h2 className="text-lg font-semibold text-[var(--text-primary)]">Settings</h2>
                            <p className="text-sm text-[var(--text-muted)]">Configure application preferences</p>
                        </div>
                    </div>
                    <button
                        onClick={onClose}
                        className="p-2 rounded-lg hover:bg-white/10 transition-colors"
                        aria-label="Close"
                    >
                        <svg className="w-5 h-5 text-[var(--text-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                        </svg>
                    </button>
                </div>

                {/* Content */}
                <div className="px-6 py-6 space-y-8">

                    {/* Performance Settings */}
                    <div>
                        <h3 className="text-sm font-medium text-[var(--text-secondary)] uppercase tracking-wider mb-4">Performance & Memory</h3>
                        <div className="space-y-4">

                            {/* Sticky Notes Toggle */}
                            <div className="flex items-center justify-between">
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Enable Sticky Notes</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Allow attaching notes to code lines. Disabling this saves memory.
                                    </p>
                                </div>
                                <label className="relative inline-flex items-center cursor-pointer">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={enableStickyNotes}
                                        onChange={(e) => setEnableStickyNotes(e.target.checked)}
                                    />
                                    <div className="w-11 h-6 bg-white/10 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-[var(--accent-color)]"></div>
                                </label>
                            </div>

                            {/* Max Result Rows */}
                            <div className="flex items-center justify-between">
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Max Result Rows</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Limit the number of rows returned per query to save memory. Set to 0 for unlimited.
                                    </p>
                                </div>
                                <input
                                    type="number"
                                    min="0"
                                    step="1000"
                                    value={maxResultRows}
                                    onChange={(e) => setMaxResultRows(parseInt(e.target.value) || 0)}
                                    className="w-24 px-3 py-1.5 bg-[var(--bg-primary)] border border-white/10 rounded-lg text-sm text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent-color)]"
                                />
                            </div>
                        </div>
                    </div>

                    <div className="h-px bg-white/5" />

                    {/* Auto Archive Settings */}
                    <div>
                        <h3 className="text-sm font-medium text-[var(--text-secondary)] uppercase tracking-wider mb-4">Auto Archive</h3>
                        <div className="space-y-4">
                            <div className="flex items-center justify-between">
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Auto Archive Days</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Automatically archive tabs that haven't been accessed for this many days.
                                    </p>
                                </div>
                                <input
                                    type="number"
                                    min="1"
                                    max="365"
                                    value={autoArchiveSettings.days_inactive}
                                    onChange={(e) => updateAutoArchiveSettings(autoArchiveSettings.enabled, parseInt(e.target.value) || 14)}
                                    className="w-24 px-3 py-1.5 bg-[var(--bg-primary)] border border-white/10 rounded-lg text-sm text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent-color)]"
                                />
                            </div>
                        </div>
                    </div>

                    <div className="h-px bg-white/5" />

                    {/* Validation Settings */}
                    <div>
                        <h3 className="text-sm font-medium text-[var(--text-secondary)] uppercase tracking-wider mb-4">SQL Validation</h3>
                        <div className="space-y-4">

                            {/* Enable Validation Toggle */}
                            <div className="flex items-center justify-between">
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Enable Real-time Validation</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Check SQL syntax and table references as you type.
                                    </p>
                                </div>
                                <label className="relative inline-flex items-center cursor-pointer">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={validationEnabled}
                                        onChange={(e) => setValidationEnabled(e.target.checked)}
                                    />
                                    <div className="w-11 h-6 bg-white/10 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-[var(--accent-color)]"></div>
                                </label>
                            </div>

                            {/* Show Warnings Toggle */}
                            <div className={`flex items-center justify-between transition-opacity ${!validationEnabled ? 'opacity-50 pointer-events-none' : ''}`}>
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Show Warnings</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Display warning messages for potential issues.
                                    </p>
                                </div>
                                <label className="relative inline-flex items-center cursor-pointer">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={validationShowWarnings}
                                        onChange={(e) => setValidationShowWarnings(e.target.checked)}
                                    />
                                    <div className="w-11 h-6 bg-white/10 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-[var(--accent-color)]"></div>
                                </label>
                            </div>

                            {/* Show Info Toggle */}
                            <div className={`flex items-center justify-between transition-opacity ${!validationEnabled ? 'opacity-50 pointer-events-none' : ''}`}>
                                <div>
                                    <label className="text-sm font-medium text-[var(--text-primary)]">Show Info Messages</label>
                                    <p className="text-xs text-[var(--text-muted)] mt-1">
                                        Display informational messages.
                                    </p>
                                </div>
                                <label className="relative inline-flex items-center cursor-pointer">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={validationShowInfo}
                                        onChange={(e) => setValidationShowInfo(e.target.checked)}
                                    />
                                    <div className="w-11 h-6 bg-white/10 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-[var(--accent-color)]"></div>
                                </label>
                            </div>

                        </div>
                    </div>

                </div>

                {/* Footer */}
                <div className="px-6 py-4 border-t border-white/10 bg-white/[0.02] flex items-center justify-center">
                    <p className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-widest opacity-50">
                        {useAppStore.getState().appInfo.name} v{useAppStore.getState().appInfo.version}
                    </p>
                </div>
            </div>
        </div>
    );
}
