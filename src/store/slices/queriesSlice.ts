import { StateCreator } from 'zustand';
import type { QueryResult, CellValue } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface QueriesSlice {
    tabQueryResults: Record<string, QueryResult[]>;
    tabExecuting: Record<string, boolean>;
    activeResultIndex: Record<string, number>;
    resultCustomNames: Record<string, Record<number, string>>;
    resultColumnOrder: Record<string, Record<number, number[]>>;
    resultsHidden: Record<string, boolean>;
    tabResultCounters: Record<string, number>;

    // Performance Settings
    enableStickyNotes: boolean;
    maxResultRows: number;

    // Cell preview panel state
    cellPreviewPanel: {
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
    };

    executeQuery: (tabId: string, query: string, selectedText?: string | null) => Promise<QueryResult[] | null>;
    executeQueryAppend: (tabId: string, query: string, selectedText?: string | null) => Promise<QueryResult[] | null>;
    executeSilentQuery: (tabId: string, query: string) => Promise<{ success: boolean; error?: string }>;
    cancelQuery: (tabId: string, queryId: string) => Promise<boolean>;
    cancelRunningQueries: (tabId: string) => Promise<number>;
    clearQueryResult: (tabId: string) => void;
    closeResult: (tabId: string, resultIndex: number) => void;

    getTabQueryResults: (tabId: string) => QueryResult[] | null;
    getActiveResultIndex: (tabId: string) => number;
    setActiveResultIndex: (tabId: string, index: number) => void;
    isTabExecuting: (tabId: string) => boolean;

    setResultCustomName: (tabId: string, resultIndex: number, name: string) => void;
    getResultCustomName: (tabId: string, resultIndex: number) => string | null;

    setResultColumnOrder: (tabId: string, resultIndex: number, order: number[]) => void;
    getResultColumnOrder: (tabId: string, resultIndex: number) => number[] | null;

    toggleResultsHidden: (tabId: string) => void;
    isResultsHidden: (tabId: string) => boolean;

    updateResultCells: (tabId: string, resultIndex: number, updates: Array<{ rowIndex: number; colIndex: number; value: CellValue }>) => void;
    reorderQueryResults: (tabId: string, fromIndex: number, toIndex: number) => void;

    setEnableStickyNotes: (enabled: boolean) => void;
    setMaxResultRows: (rows: number) => void;

    showCellPreview: (
        tabId: string,
        resultIndex: number,
        rowIndex: number,
        colIndex: number,
        value: CellValue,
        columnName: string,
        dataType: string
    ) => void;
    hideCellPreview: () => void;
    setCellPreviewWidth: (width: number) => void;
    setCellPreviewWidthImmediate: (width: number) => void;
    setCellPreviewFormatter: (formatter: 'auto' | 'json' | 'xml' | 'plain') => void;
}

export const createQueriesSlice: StateCreator<AppState, [], [], QueriesSlice> = (set, get) => ({
    tabQueryResults: {},
    tabExecuting: {},
    activeResultIndex: {},
    resultCustomNames: {},
    resultColumnOrder: {},
    resultsHidden: {},
    tabResultCounters: {},
    enableStickyNotes: true,
    maxResultRows: 5000,

    cellPreviewPanel: {
        visible: false,
        width: (() => {
            try {
                const stored = localStorage.getItem('larik-cell-preview-width');
                return stored ? parseInt(stored, 10) : 500;
            } catch {
                return 500;
            }
        })(),
        selectedCell: null,
        formatterType: 'auto'
    },

    executeQuery: async (tabId, query, selectedText) => {
        // Basic validation check
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            get().addToast({ type: 'error', message: 'No active space' });
            return null;
        }

        set((state) => ({
            tabExecuting: { ...state.tabExecuting, [tabId]: true }
        }));

        try {
            const activeTab = get().tabs.find(t => t.id === tabId);
            const activeSpace = get().spaces.find(s => s.id === spaceId);

            const database = activeTab?.database || activeSpace?.connection_database;

            const results = await api.executeQuery(
                spaceId,
                query,
                database,
                selectedText
            );

            set((state) => {
                const currentResults = state.tabQueryResults[tabId] || [];
                // If we have no results, default to 0. If we have results, use the active index.
                const activeIndex = currentResults.length > 0 ? (state.activeResultIndex[tabId] ?? 0) : 0;

                // Clone current results
                let newResults = [...currentResults];

                // If currently empty, just set results.
                // Otherwise replace the active result with new results.
                if (currentResults.length === 0) {
                    newResults = results;
                } else {
                    // Remove the active result and insert the new one(s)
                    newResults.splice(activeIndex, 1, ...results);
                }

                // If we replaced 1 item with N items, subsquent items need to be shifted by N-1
                const shiftAmount = results.length - 1;

                // Helper to shift metadata keys
                const shiftMap = <T>(map: Record<number, T> | undefined): Record<number, T> => {
                    if (!map) return {};
                    const newMap: Record<number, T> = {};
                    Object.entries(map).forEach(([k, v]) => {
                        const idx = parseInt(k, 10);
                        if (idx < activeIndex) {
                            // Before active index: keep as is
                            newMap[idx] = v;
                        } else if (idx > activeIndex) {
                            // After active index: shift
                            newMap[idx + shiftAmount] = v;
                        }
                        // matched idx is dropped (reset for new result)
                    });
                    return newMap;
                };

                const newCustomNames = shiftMap(state.resultCustomNames[tabId]);
                const newColumnOrders = shiftMap(state.resultColumnOrder[tabId]);

                // Assign displayId to new results
                const currentCounter = state.tabResultCounters[tabId] || 0;
                results.forEach((r, i) => {
                    r.displayId = currentCounter + i + 1;
                });
                const newCounter = currentCounter + results.length;

                return {
                    tabQueryResults: { ...state.tabQueryResults, [tabId]: newResults },
                    // Keep focus on the same position (start of the new results)
                    activeResultIndex: { ...state.activeResultIndex, [tabId]: activeIndex },
                    resultCustomNames: { ...state.resultCustomNames, [tabId]: newCustomNames },
                    resultColumnOrder: { ...state.resultColumnOrder, [tabId]: newColumnOrders },
                    tabResultCounters: { ...state.tabResultCounters, [tabId]: newCounter },
                    tabExecuting: { ...state.tabExecuting, [tabId]: false }
                };
            });

            return results;
        } catch (error) {
            console.error('Query execution failed:', error);
            set((state) => ({
                tabExecuting: { ...state.tabExecuting, [tabId]: false }
            }));

            get().addToast({
                type: 'error',
                message: error instanceof Error ? error.message : 'Query failed'
            });
            return null;
        }
    },

    executeQueryAppend: async (tabId, query, selectedText) => {
        // Similar to executeQuery but appends results
        const spaceId = get().activeSpaceId;
        if (!spaceId) return null;

        set((state) => ({
            tabExecuting: { ...state.tabExecuting, [tabId]: true }
        }));

        try {
            const activeTab = get().tabs.find(t => t.id === tabId);
            const activeSpace = get().spaces.find(s => s.id === spaceId);
            const database = activeTab?.database || activeSpace?.connection_database;

            const newResults = await api.executeQuery(
                spaceId,
                query,
                database,
                selectedText
            );

            set((state) => {
                const currentResults = state.tabQueryResults[tabId] || [];
                const currentCounter = state.tabResultCounters[tabId] || 0;

                // Assign displayId to new results
                newResults.forEach((r, i) => {
                    r.displayId = currentCounter + i + 1;
                });
                const newCounter = currentCounter + newResults.length;

                // Append new results
                const combinedResults = [...currentResults, ...newResults];
                // Set active index to the start of new results
                const newActiveIndex = currentResults.length;

                return {
                    tabQueryResults: { ...state.tabQueryResults, [tabId]: combinedResults },
                    activeResultIndex: { ...state.activeResultIndex, [tabId]: newActiveIndex },
                    tabResultCounters: { ...state.tabResultCounters, [tabId]: newCounter },
                    tabExecuting: { ...state.tabExecuting, [tabId]: false }
                };
            });
            return newResults;
        } catch (error) {
            console.error('Query append execution failed:', error);
            set((state) => ({
                tabExecuting: { ...state.tabExecuting, [tabId]: false }
            }));
            return null;
        }
    },

    executeSilentQuery: async (tabId, query) => {
        // Execute without updating UI state (for background checks etc)
        const spaceId = get().activeSpaceId;
        if (!spaceId) return { success: false, error: 'No active space' };

        try {
            const activeTab = get().tabs.find(t => t.id === tabId);
            const activeSpace = get().spaces.find(s => s.id === spaceId);
            const database = activeTab?.database || activeSpace?.connection_database;

            await api.executeQuery(spaceId, query, database);
            return { success: true };
        } catch (error) {
            return { success: false, error: String(error) };
        }
    },

    cancelQuery: async (_tabId, queryId) => {
        try {
            return await api.cancelQuery(queryId);
        } catch (error) {
            console.error('Failed to cancel query:', error);
            return false;
        }
    },

    cancelRunningQueries: async (_tabId) => {
        try {
            // Find all running queries for this tab... logic simplified as API handles it?
            // Need connection ID?
            // The API `cancelQueriesForConnection` might be needed
            // But for now let's assume `cancelQuery` is enough if we track IDs
            // Actually backend `cancel_query` takes a query_id. 
            // Do we track query IDs separately? Only in results.
            // If query is strictly running, we might not have result yet?
            // Backend T019 implementation details...
            return 0;
        } catch (error) {
            return 0;
        }
    },

    clearQueryResult: (tabId) => {
        set((state) => {
            const { [tabId]: _, ...restResults } = state.tabQueryResults;
            const { [tabId]: __, ...restIndices } = state.activeResultIndex;
            const { [tabId]: ___, ...restCounters } = state.tabResultCounters;
            return {
                tabQueryResults: restResults,
                activeResultIndex: restIndices,
                tabResultCounters: restCounters
            };
        });
    },

    closeResult: (tabId, resultIndex) => {
        set((state) => {
            const currentResults = state.tabQueryResults[tabId] || [];
            const newResults = [...currentResults];
            newResults.splice(resultIndex, 1);

            // Update active index if needed
            let newActiveIndex = state.activeResultIndex[tabId] || 0;
            if (newActiveIndex >= newResults.length) {
                newActiveIndex = Math.max(0, newResults.length - 1);
            }

            return {
                tabQueryResults: { ...state.tabQueryResults, [tabId]: newResults },
                activeResultIndex: { ...state.activeResultIndex, [tabId]: newActiveIndex },
                ...(newResults.length === 0 ? {
                    tabResultCounters: { ...state.tabResultCounters, [tabId]: 0 }
                } : {})
            };
        });
    },

    getTabQueryResults: (tabId) => {
        return get().tabQueryResults[tabId] || null;
    },

    getActiveResultIndex: (tabId) => {
        return get().activeResultIndex[tabId] || 0;
    },

    setActiveResultIndex: (tabId, index) => {
        set((state) => ({
            activeResultIndex: { ...state.activeResultIndex, [tabId]: index }
        }));
    },

    isTabExecuting: (tabId) => {
        return get().tabExecuting[tabId] || false;
    },

    setResultCustomName: (tabId, resultIndex, name) => {
        set((state) => ({
            resultCustomNames: {
                ...state.resultCustomNames,
                [tabId]: {
                    ...(state.resultCustomNames[tabId] || {}),
                    [resultIndex]: name
                }
            }
        }));
    },

    getResultCustomName: (tabId, resultIndex) => {
        const names = get().resultCustomNames[tabId];
        return names ? names[resultIndex] : null;
    },

    setResultColumnOrder: (tabId, resultIndex, order) => {
        set((state) => ({
            resultColumnOrder: {
                ...state.resultColumnOrder,
                [tabId]: {
                    ...(state.resultColumnOrder[tabId] || {}),
                    [resultIndex]: order
                }
            }
        }));
    },

    getResultColumnOrder: (tabId, resultIndex) => {
        const orders = get().resultColumnOrder[tabId];
        return orders ? orders[resultIndex] : null;
    },

    toggleResultsHidden: (tabId) => {
        set((state) => ({
            resultsHidden: {
                ...state.resultsHidden,
                [tabId]: !state.resultsHidden[tabId]
            }
        }));
    },

    isResultsHidden: (tabId) => {
        return get().resultsHidden[tabId] || false;
    },

    updateResultCells: (tabId, resultIndex, updates) => {
        set((state) => {
            const currentResults = state.tabQueryResults[tabId];
            if (!currentResults) return state;

            const newResults = [...currentResults];
            const targetresult = { ...newResults[resultIndex] };
            const newRows = [...targetresult.rows];

            updates.forEach(({ rowIndex, colIndex, value }) => {
                if (newRows[rowIndex]) {
                    const newRow = [...newRows[rowIndex]];
                    newRow[colIndex] = value;
                    newRows[rowIndex] = newRow;
                }
            });

            targetresult.rows = newRows;
            newResults[resultIndex] = targetresult;

            return {
                tabQueryResults: {
                    ...state.tabQueryResults,
                    [tabId]: newResults
                }
            };
        });
    },

    reorderQueryResults: (tabId, fromIndex, toIndex) => {
        set((state) => {
            const currentResults = state.tabQueryResults[tabId];
            if (!currentResults) return state;

            const newResults = [...currentResults];
            const [moved] = newResults.splice(fromIndex, 1);
            newResults.splice(toIndex, 0, moved);

            // Helper to reorder map keys
            const reorderMap = <T>(map: Record<number, T> | undefined): Record<number, T> => {
                if (!map) return {};
                const newMap: Record<number, T> = {};
                const items = Object.entries(map).map(([k, v]) => ({ index: parseInt(k, 10), value: v }));

                // Adjust indices
                items.forEach(item => {
                    const idx = item.index;
                    if (idx === fromIndex) {
                        item.index = toIndex;
                    } else if (fromIndex < toIndex) {
                        // Moving forward: items between from and to shift down
                        if (idx > fromIndex && idx <= toIndex) {
                            item.index = idx - 1;
                        }
                    } else {
                        // Moving backward: items between to and from shift up
                        if (idx >= toIndex && idx < fromIndex) {
                            item.index = idx + 1;
                        }
                    }
                });

                items.forEach(item => {
                    newMap[item.index] = item.value;
                });
                return newMap;
            };

            const newCustomNames = reorderMap(state.resultCustomNames[tabId]);
            const newColumnOrders = reorderMap(state.resultColumnOrder[tabId]);

            return {
                tabQueryResults: {
                    ...state.tabQueryResults,
                    [tabId]: newResults
                },
                resultCustomNames: {
                    ...state.resultCustomNames,
                    [tabId]: newCustomNames
                },
                resultColumnOrder: {
                    ...state.resultColumnOrder,
                    [tabId]: newColumnOrders
                },
                activeResultIndex: {
                    ...state.activeResultIndex,
                    [tabId]: toIndex // Focus moved result
                }
            };
        });
    },

    setEnableStickyNotes: (enabled) => set({ enableStickyNotes: enabled }),
    setMaxResultRows: (rows) => set({ maxResultRows: rows }),

    showCellPreview: (tabId, resultIndex, rowIndex, colIndex, value, columnName, dataType) => {
        set((state) => ({
            cellPreviewPanel: {
                ...state.cellPreviewPanel,
                visible: true,
                selectedCell: { tabId, resultIndex, rowIndex, colIndex, value, columnName, dataType }
            }
        }));
    },

    hideCellPreview: () => {
        set((state) => ({
            cellPreviewPanel: { ...state.cellPreviewPanel, visible: false, selectedCell: null }
        }));
    },

    setCellPreviewWidth: (width) => {
        set((state) => ({
            cellPreviewPanel: { ...state.cellPreviewPanel, width }
        }));
        try {
            localStorage.setItem('larik-cell-preview-width', width.toString());
        } catch {
            // ignore
        }
    },

    setCellPreviewWidthImmediate: (width) => {
        set((state) => ({
            cellPreviewPanel: { ...state.cellPreviewPanel, width }
        }));
    },

    setCellPreviewFormatter: (formatter) => {
        set((state) => ({
            cellPreviewPanel: { ...state.cellPreviewPanel, formatterType: formatter }
        }));
    }
});
