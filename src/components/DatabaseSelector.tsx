import { useState, useEffect, useRef } from 'react';
import { useAppStore } from '../store';

interface DatabaseSelectorProps {
    isCompact?: boolean;
}

export function DatabaseSelector({ isCompact = false }: DatabaseSelectorProps) {
    const activeTabId = useAppStore(s => s.activeTabId);
    const tabs = useAppStore(s => s.tabs);
    const spaces = useAppStore(s => s.spaces);
    const activeSpaceId = useAppStore(s => s.activeSpaceId);
    const isConnected = useAppStore(s => s.spaceConnectionStatus?.is_connected ?? false);
    const spaceDatabases = useAppStore(s => s.spaceDatabases);
    const databasesLoading = useAppStore(s => s.databasesLoading);
    const updateTabDatabase = useAppStore(s => s.updateTabDatabase);

    const [isOpen, setIsOpen] = useState(false);
    const dropdownRef = useRef<HTMLDivElement>(null);

    const activeSpace = spaces.find(s => s.id === activeSpaceId);
    const activeTab = tabs.find(t => t.id === activeTabId);
    const spaceColor = activeSpace?.color || '#6366f1';

    // Close dropdown when clicking outside
    useEffect(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
                setIsOpen(false);
            }
        };
        if (isOpen) {
            document.addEventListener('mousedown', handleClickOutside);
        }
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, [isOpen]);

    const handleSelectDatabase = (db: string | null) => {
        if (activeTab) {
            updateTabDatabase(activeTab.id, db);
        }
        setIsOpen(false);
    };

    const currentDatabaseValue = activeTab?.database || activeSpace?.connection_database || 'Default DB';

    return (
        <div
            className={`text-xs flex items-center gap-2 bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-lg hover:bg-[var(--bg-active)] transition-all group relative ${isCompact ? 'px-1.5 py-0.5 h-6 max-w-[220px] min-w-[80px]' : 'px-2 py-1 h-7 min-w-[140px]'}`}
            style={{
                boxShadow: isConnected ? `0 0 0 1px ${spaceColor}10` : 'none'
            }}
        >
            <svg
                className={`${isCompact ? 'w-3 h-3' : 'w-4 h-4'} flex-shrink-0 transition-colors`}
                style={{ color: isConnected ? spaceColor : 'var(--text-muted)' }}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
            >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4" />
            </svg>

            {isConnected && spaceDatabases.length > 0 ? (
                <div ref={dropdownRef} className="flex-1 relative min-w-0 h-full flex items-center">
                    <button
                        onClick={() => !databasesLoading && setIsOpen(!isOpen)}
                        disabled={databasesLoading}
                        className="w-full h-full flex items-center justify-between gap-1 text-left disabled:opacity-50"
                    >
                        <span className={`font-medium text-[var(--text-primary)] flex-1 overflow-hidden relative group/marquee max-w-full ${isCompact ? 'text-[10px]' : 'text-xs'}`}>
                            <span
                                key={currentDatabaseValue}
                                ref={(el) => {
                                    if (el && el.parentElement) {
                                        const parent = el.parentElement;
                                        setTimeout(() => {
                                            const parentWidth = parent.offsetWidth;
                                            const scrollWidth = el.scrollWidth;
                                            if (scrollWidth > parentWidth) {
                                                parent.style.setProperty('--marquee-width', `${parentWidth}px`);
                                                const duration = Math.max(3, scrollWidth / 30);
                                                parent.style.setProperty('--marquee-duration', `${duration}s`);
                                                parent.classList.add('animate-marquee-hover');
                                            } else {
                                                parent.classList.remove('animate-marquee-hover');
                                            }
                                        }, 50);
                                    }
                                }}
                                className="marquee-content inline-block whitespace-nowrap"
                            >
                                {currentDatabaseValue}
                            </span>
                        </span>
                        <svg
                            className={`${isCompact ? 'w-3 h-3' : 'w-4 h-4'} flex-shrink-0 opacity-40 group-hover:opacity-100 transition-transform ${isOpen ? 'rotate-180' : ''}`}
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24"
                        >
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                        </svg>
                    </button>

                    {isOpen && (
                        <div
                            className={`absolute top-full mt-1.5 z-[100] py-1 rounded-lg border border-[var(--border-color)] shadow-2xl overflow-hidden animate-in fade-in slide-in-from-top-1 duration-150 bg-[var(--bg-secondary)] backdrop-blur-xl ${isCompact ? 'left-[-30px] w-[180px]' : 'left-[-10px] right-[-10px]'}`}
                            style={{ maxHeight: '280px', overflowY: 'auto' }}
                        >
                            <button
                                onClick={() => handleSelectDatabase(null)}
                                className={`w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 transition-colors ${!activeTab?.database
                                    ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                                    : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]'
                                    }`}
                            >
                                <span className="truncate">{activeSpace?.connection_database || 'Default DB'}</span>
                            </button>
                            <div className="my-1 mx-2 border-t border-[var(--border-color)] opacity-50" />
                            {spaceDatabases.map((db) => (
                                <button
                                    key={db.name}
                                    onClick={() => db.hasAccess ? handleSelectDatabase(db.name) : undefined}
                                    disabled={!db.hasAccess}
                                    title={!db.hasAccess ? 'You do not have access to this database' : undefined}
                                    className={`w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 transition-colors ${!db.hasAccess
                                            ? 'opacity-40 cursor-not-allowed text-[var(--text-muted)]'
                                            : activeTab?.database === db.name
                                                ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                                                : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]'
                                        }`}
                                >
                                    {db.hasAccess ? (
                                        <svg className="w-3 h-3 flex-shrink-0 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                                        </svg>
                                    ) : (
                                        <svg className="w-3 h-3 flex-shrink-0 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                                        </svg>
                                    )}
                                    <span className="truncate">{db.name}</span>
                                    {!db.hasAccess && (
                                        <span className="ml-auto text-[9px] text-[var(--text-muted)] shrink-0">No access</span>
                                    )}
                                </button>
                            ))}
                        </div>
                    )}
                </div>
            ) : (
                <span className={`flex-1 text-[var(--text-muted)] truncate ${isCompact ? 'text-[10px]' : 'text-xs'}`}>
                    {isConnected ? currentDatabaseValue : 'Not connected'}
                </span>
            )}
        </div>
    );
}
