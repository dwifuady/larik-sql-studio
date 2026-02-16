import { useState, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useAppStore } from '../store';
import * as api from '../api';
import type { Tab } from '../types';

export function GlobalSearch() {
    const [query, setQuery] = useState('');
    const [results, setResults] = useState<Tab[]>([]);
    const [isOpen, setIsOpen] = useState(false);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const inputRef = useRef<HTMLInputElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const dropdownRef = useRef<HTMLDivElement>(null);

    const { setActiveSpace, setActiveTab, spaces } = useAppStore();

    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'f') {
                e.preventDefault();
                setIsOpen(true);
                setTimeout(() => inputRef.current?.focus(), 50);
            }

            if (isOpen && e.key === 'Escape') {
                setIsOpen(false);
                setQuery('');
                inputRef.current?.blur();
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [isOpen]);

    useEffect(() => {
        const search = async () => {
            if (!query.trim()) {
                setResults([]);
                return;
            }

            try {
                const tabs = await api.searchTabs(query);
                setResults(tabs);
                setSelectedIndex(0);
            } catch (error) {
                console.error('Failed to search tabs:', error);
            }
        };

        const timeoutId = setTimeout(search, 300);
        return () => clearTimeout(timeoutId);
    }, [query]);

    // Handle outside click to close
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            // Check if click is outside both container (input) and dropdown (portal)
            if (
                containerRef.current &&
                !containerRef.current.contains(event.target as Node) &&
                dropdownRef.current &&
                !dropdownRef.current.contains(event.target as Node)
            ) {
                setIsOpen(false);
            }
        };

        if (isOpen) {
            document.addEventListener('mousedown', handleClickOutside);
        }
        return () => {
            document.removeEventListener('mousedown', handleClickOutside);
        };
    }, [isOpen]);

    const handleSelect = async (tab: Tab) => {
        setIsOpen(false);
        setQuery('');
        inputRef.current?.blur();

        // 1. Switch space if needed
        if (tab.space_id) {
            await setActiveSpace(tab.space_id);
        }

        // 2. Switch tab
        // Small delay to allow space/tabs to load
        setTimeout(() => {
            setActiveTab(tab.id);
        }, 100);
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelectedIndex(prev => (prev + 1) % results.length);
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelectedIndex(prev => (prev - 1 + results.length) % results.length);
        } else if (e.key === 'Enter') {
            e.preventDefault();
            if (results[selectedIndex]) {
                handleSelect(results[selectedIndex]);
            }
        }
    };

    const getSpaceName = (spaceId: string) => {
        return spaces.find(s => s.id === spaceId)?.name || 'Unknown Space';
    };

    // Spotlight-style modal content
    const searchModal = isOpen ? (
        <div className="fixed inset-0 z-[9999] flex items-start justify-center pt-[20vh]" style={{ pointerEvents: 'auto' }}>
            {/* Backdrop */}
            <div
                className="absolute inset-0 bg-black/20 backdrop-blur-sm transition-opacity"
                onClick={() => setIsOpen(false)}
            />

            {/* Modal Container */}
            <div
                ref={dropdownRef}
                className="relative w-[36rem] flex flex-col bg-[var(--bg-secondary)]/90 backdrop-blur-xl border border-[var(--border-color)] rounded-xl shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200"
                onClick={(e) => e.stopPropagation()}
            >
                {/* Search Input Area */}
                <div className="flex items-center gap-3 px-4 py-3 border-b border-[var(--border-color)]">
                    <svg className="w-5 h-5 text-[var(--accent-color)] flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                    </svg>
                    <input
                        ref={inputRef}
                        type="text"
                        className="flex-1 pl-3 pr-8 py-1.5 text-sm bg-[var(--bg-hover)] border border-[var(--border-color)] rounded-md text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:border-[var(--accent-color)] focus:bg-[var(--bg-active)] transition-colors"
                        placeholder="Search tabs, schema, or queries..."
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        onKeyDown={handleKeyDown}
                        autoFocus
                    />
                    <kbd className="px-2 py-1 text-xs text-[var(--text-muted)] bg-[var(--bg-hover)] rounded border border-[var(--border-color)]">
                        ESC
                    </kbd>
                </div>

                {/* Results List */}
                <div className="max-h-[60vh] overflow-y-auto overflow-x-hidden">
                    {query && results.length > 0 ? (
                        <div className="p-2">
                            {results.map((tab, index) => (
                                <button
                                    key={tab.id}
                                    className={`w-full text-left px-3 py-3 text-sm flex items-center gap-3 rounded-lg transition-all duration-150 group
                                        ${index === selectedIndex ? 'bg-[var(--accent-color)]/10 text-[var(--accent-color)]' : 'text-[var(--text-primary)] hover:bg-[var(--bg-hover)]'}
                                    `}
                                    onClick={() => handleSelect(tab)}
                                    onMouseEnter={() => setSelectedIndex(index)}
                                >
                                    {/* Icon based on tab type */}
                                    <div className={`p-1.5 rounded-md ${index === selectedIndex ? 'bg-[var(--accent-color)]/20' : 'bg-[var(--bg-tertiary)] group-hover:bg-[var(--bg-tertiary)]/80'}`}>
                                        {tab.tab_type === 'query' ? (
                                            <svg className={`w-4 h-4 ${index === selectedIndex ? 'text-[var(--accent-color)]' : 'text-[var(--text-muted)]'}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                                            </svg>
                                        ) : tab.tab_type === 'schema' ? (
                                            <svg className="w-4 h-4 text-[var(--accent-color)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
                                            </svg>
                                        ) : (
                                            <svg className={`w-4 h-4 ${index === selectedIndex ? 'text-[var(--accent-color)]' : 'text-[var(--text-muted)]'}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                                            </svg>
                                        )}
                                    </div>

                                    <div className="flex-1 min-w-0">
                                        <div className="flex items-center justify-between">
                                            <span className="font-medium truncate">{tab.title}</span>
                                            <span className={`text-[10px] uppercase tracking-wider font-semibold ml-2 whitespace-nowrap px-1.5 py-0.5 rounded ${index === selectedIndex ? 'bg-[var(--accent-color)]/20 text-[var(--accent-color)]' : 'bg-[var(--bg-tertiary)] text-[var(--text-muted)]'}`}>
                                                {getSpaceName(tab.space_id)}
                                            </span>
                                        </div>
                                        {tab.content && (
                                            <div className={`text-xs truncate transition-colors mt-0.5 ${index === selectedIndex ? 'text-[var(--accent-color)]/70' : 'text-[var(--text-muted)]'}`}>
                                                {tab.content.substring(0, 100)}
                                            </div>
                                        )}
                                    </div>

                                    {index === selectedIndex && (
                                        <div className="text-[var(--accent-color)]">
                                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 5l7 7-7 7" />
                                            </svg>
                                        </div>
                                    )}
                                </button>
                            ))}
                        </div>
                    ) : query && results.length === 0 ? (
                        <div className="p-12 text-center text-[var(--text-muted)]">
                            <p>No matches found for "{query}"</p>
                        </div>
                    ) : (
                        <div className="p-12 text-center text-[var(--text-muted)] opacity-50">
                            Type to search across all your tabs
                        </div>
                    )}
                </div>

                {/* Footer */}
                <div className="px-4 py-2 border-t border-[var(--border-color)] bg-[var(--bg-tertiary)]/50 text-[10px] text-[var(--text-muted)] flex justify-between">
                    <span>Search Tab Titles & Content</span>
                    <div className="flex gap-3">
                        <span className="flex items-center gap-1"><kbd className="font-sans px-1 rounded bg-[var(--bg-primary)] border border-[var(--border-color)]">↵</kbd> select</span>
                        <span className="flex items-center gap-1"><kbd className="font-sans px-1 rounded bg-[var(--bg-primary)] border border-[var(--border-color)]">↑↓</kbd> navigate</span>
                    </div>
                </div>
            </div>
        </div>
    ) : null;

    // Trigger button in titlebar
    return (
        <div ref={containerRef} className="relative mx-auto w-full max-w-lg pointer-events-auto h-full flex items-center justify-center">
            {/* The Trigger Input - Only visible when NOT open */}
            <div
                className={`relative flex items-center transition-all duration-200 group w-full ${isOpen ? 'opacity-0 pointer-events-none' : 'opacity-100'}`}
                onClick={() => setIsOpen(true)}
            >
                <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none z-10">
                    <svg className="h-3.5 w-3.5 text-[var(--text-muted)] group-hover:text-[var(--text-secondary)] transition-colors" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                    </svg>
                </div>

                {/* Simulated Input (Button) */}
                <div
                    className="w-full h-6 bg-[var(--bg-primary)]/40 hover:bg-[var(--bg-primary)]/60 text-[var(--text-muted)] hover:text-[var(--text-primary)] border border-transparent hover:border-[var(--border-subtle)] rounded-md py-0.5 pl-9 pr-12 text-xs shadow-sm transition-all duration-200 cursor-text flex items-center"
                >
                    <span className="truncate opacity-70">Search tabs...</span>
                </div>

                <div className="absolute inset-y-0 right-0 pr-2 flex items-center pointer-events-none">
                    <span className="text-[9px] px-1 py-0.5 rounded border border-[var(--border-subtle)] bg-[var(--bg-tertiary)]/50 text-[var(--text-muted)]">
                        ⌘F
                    </span>
                </div>
            </div>

            {createPortal(searchModal, document.body)}
        </div>
    );
}
