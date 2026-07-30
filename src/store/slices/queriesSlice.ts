import { StateCreator } from 'zustand';
import type { QueryResult, CellValue } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';
import type { ReferenceRequest } from './referencePreviewSlice';

/** Which tab the cell preview panel is showing. */
export type CellPreviewTab = 'value' | 'reference';

/** The cell a query tab is previewing. */
export interface CellPreviewSelectedCell {
    tabId: string;
    resultIndex: number;
    rowIndex: number;
    colIndex: number;
    value: CellValue;
    columnName: string;
    dataType: string;
    /** Foreign key lookup for this cell, when the column references another table. */
    referenceRequest: ReferenceRequest | null;
}

/** One query tab's preview state. */
export interface CellPreviewEntry {
    selectedCell: CellPreviewSelectedCell;
    activeTab: CellPreviewTab;
    /** query_id of the result the cell came from, to detect a re-run. */
    queryId: string | null;
}

export interface QueriesSlice {
    tabQueryResults: Record<string, QueryResult[]>;
    tabExecuting: Record<string, boolean>;
    activeResultIndex: Record<string, number>;
    resultCustomNames: Record<string, Record<number, string>>;
    resultColumnOrder: Record<string, Record<number, number[]>>;
    resultScrollPosition: Record<string, Record<number, { top: number; left: number }>>;
    resultsHidden: Record<string, boolean>;
    tabResultCounters: Record<string, number>;

    // Performance Settings
    enableStickyNotes: boolean;
    maxResultRows: number;

    /**
     * Cell preview panel. Width and formatter are shared; what is being
     * previewed is remembered per tab, so switching tab or space leaves each
     * tab's preview intact instead of closing it.
     */
    cellPreviewPanel: {
        width: number;
        formatterType: 'auto' | 'json' | 'xml' | 'plain';
        byTab: Record<string, CellPreviewEntry>;
    };

    executeQuery: (tabId: string, query: string, selectedText?: string | null, maxRowsOverride?: number) => Promise<QueryResult[] | null>;
    executeQueryAppend: (tabId: string, query: string, selectedText?: string | null, maxRowsOverride?: number) => Promise<QueryResult[] | null>;
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

    setResultScrollPosition: (tabId: string, resultIndex: number, top: number, left: number) => void;
    getResultScrollPosition: (tabId: string, resultIndex: number) => { top: number; left: number } | null;

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
        dataType: string,
        options?: {
            referenceRequest?: ReferenceRequest | null;
            tab?: CellPreviewTab;
            queryId?: string | null;
        }
    ) => void;
    /** Close the preview for one tab (defaults to the active tab). */
    hideCellPreview: (tabId?: string) => void;
    setCellPreviewWidth: (width: number) => void;
    setCellPreviewWidthImmediate: (width: number) => void;
    setCellPreviewFormatter: (formatter: 'auto' | 'json' | 'xml' | 'plain') => void;
    setCellPreviewTab: (tabId: string, tab: CellPreviewTab) => void;
}

export const createQueriesSlice: StateCreator<AppState, [], [], QueriesSlice> = (set, get) => ({
    tabQueryResults: {},
    tabExecuting: {},
    activeResultIndex: {},
    resultCustomNames: {},
    resultColumnOrder: {},
    resultScrollPosition: {},
    resultsHidden: {},
    tabResultCounters: {},
    enableStickyNotes: true,
    maxResultRows: 5000,

    cellPreviewPanel: {
        width: (() => {
            try {
                const stored = localStorage.getItem('larik-cell-preview-width');
                return stored ? parseInt(stored, 10) : 500;
            } catch {
                return 500;
            }
        })(),
        formatterType: 'auto',
        byTab: {}
    },

    executeQuery: async (tabId, query, selectedText, maxRowsOverride) => {
        // Basic validation check
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            get().addToast({ type: 'error', message: 'No active space' });
            return null;
        }

        set((state) => ({
            tabExecuting: { ...state.tabExecuting, [tabId]: true }
        }));

        const isPasswordExpiredError = (errorMessage: string): boolean => {
            return (
                errorMessage.toLowerCase().includes('password expired') ||
                errorMessage.toLowerCase().includes('password must be changed') ||
                errorMessage.toLowerCase().includes('password change required')
            );
        };

        const handlePasswordExpired = () => {
            // Stop loading state immediately (tabId is captured from outer closure)
            set((state) => ({
                tabExecuting: { ...state.tabExecuting, [tabId]: false }
            }));
            get().addToast({
                type: 'error',
                message: 'Password expired. Please update your password in SQL Server.',
                duration: 8000
            });
        };

        const executeWithRetry = async (retryCount = 0): Promise<QueryResult[] | null> => {
            try {
                const activeTab = get().tabs.find(t => t.id === tabId);
                const activeSpace = get().spaces.find(s => s.id === spaceId);

                const database = activeTab?.database || activeSpace?.connection_database;

                const maxRows = maxRowsOverride ?? get().maxResultRows;

                const results = await api.executeQuery(
                    spaceId,
                    query,
                    database,
                    selectedText,
                    maxRows
                );

                // Check results for embedded password-expired errors
                // (happens in batch execution where error is returned inside QueryResult, not as exception)
                for (const result of results) {
                    if (result.error && isPasswordExpiredError(result.error)) {
                        handlePasswordExpired();
                    }
                }

                return results;
            } catch (error: any) {
                const errorMessage = String(error?.message || error || '');

                // Do NOT attempt reconnection for password-expired errors
                if (isPasswordExpiredError(errorMessage)) {
                    handlePasswordExpired();
                    return null;
                }

                const isConnectionError =
                    errorMessage.includes('Transport level error') ||
                    errorMessage.includes('Connection reset') ||
                    errorMessage.includes('broken pipe') ||
                    errorMessage.includes('Communication link failure') ||
                    errorMessage.includes('TCP Provider') ||
                    errorMessage.includes('Force Disconnect');

                if (isConnectionError && retryCount < 1) {
                    // Attempt to reconnect
                    console.log('Connection lost, attempting to reconnect...', errorMessage);
                    get().addToast({ type: 'info', message: 'Connection lost. Reconnecting...' });

                    try {
                        const connected = await api.connectToSpace(spaceId);
                        if (connected) {
                            // Retry the query
                            return executeWithRetry(retryCount + 1);
                        }
                    } catch (reconnectError) {
                        console.error('Reconnection failed:', reconnectError);
                    }
                }
                throw error;
            }
        };

        try {
            const results = await executeWithRetry();

            if (!results) return null;

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
                const newScrollPositions = shiftMap(state.resultScrollPosition[tabId]);

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
                    resultScrollPosition: { ...state.resultScrollPosition, [tabId]: newScrollPositions },
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

    executeQueryAppend: async (tabId, query, selectedText, maxRowsOverride) => {
        // Similar to executeQuery but appends results
        const spaceId = get().activeSpaceId;
        if (!spaceId) return null;

        set((state) => ({
            tabExecuting: { ...state.tabExecuting, [tabId]: true }
        }));

        const isPasswordExpiredError = (errorMessage: string): boolean => {
            return (
                errorMessage.toLowerCase().includes('password expired') ||
                errorMessage.toLowerCase().includes('password must be changed') ||
                errorMessage.toLowerCase().includes('password change required')
            );
        };

        const handlePasswordExpired = () => {
            // Stop loading state immediately
            set((state) => ({
                tabExecuting: { ...state.tabExecuting, [tabId]: false }
            }));
            get().addToast({
                type: 'error',
                message: 'Password expired. Please update your password in SQL Server.',
                duration: 8000
            });
        };

        const executeWithRetry = async (retryCount = 0): Promise<QueryResult[] | null> => {
            try {
                const activeTab = get().tabs.find(t => t.id === tabId);
                const activeSpace = get().spaces.find(s => s.id === spaceId);
                const database = activeTab?.database || activeSpace?.connection_database;

                const maxRows = maxRowsOverride ?? get().maxResultRows;

                const newResults = await api.executeQuery(
                    spaceId,
                    query,
                    database,
                    selectedText,
                    maxRows
                );

                // Check results for embedded password-expired errors
                for (const result of newResults) {
                    if (result.error && isPasswordExpiredError(result.error)) {
                        handlePasswordExpired();
                    }
                }

                return newResults;
            } catch (error: any) {
                const errorMessage = String(error?.message || error || '');

                // Do NOT attempt reconnection for password-expired errors
                if (isPasswordExpiredError(errorMessage)) {
                    handlePasswordExpired();
                    return null;
                }

                const isConnectionError =
                    errorMessage.includes('Transport level error') ||
                    errorMessage.includes('Connection reset') ||
                    errorMessage.includes('broken pipe') ||
                    errorMessage.includes('Communication link failure') ||
                    errorMessage.includes('TCP Provider') ||
                    errorMessage.includes('Force Disconnect');

                if (isConnectionError && retryCount < 1) {
                    console.log('Connection lost (append), attempting to reconnect...', errorMessage);
                    get().addToast({ type: 'info', message: 'Connection lost. Reconnecting...' });

                    try {
                        const connected = await api.connectToSpace(spaceId);
                        if (connected) {
                            return executeWithRetry(retryCount + 1);
                        }
                    } catch (reconnectError) {
                        console.error('Reconnection failed:', reconnectError);
                    }
                }
                throw error;
            }
        };

        try {
            const newResults = await executeWithRetry();
            if (!newResults) return null;

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

            get().addToast({
                type: 'error',
                message: error instanceof Error ? error.message : 'Query failed'
            });

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
            // Use the active space ID as the connection ID (1:1 model)
            const spaceId = get().activeSpaceId;
            if (!spaceId) return 0;
            return await api.cancelQueriesForConnection(spaceId);
        } catch (error) {
            console.error('Failed to cancel running queries:', error);
            return 0;
        }
    },

    clearQueryResult: (tabId) => {
        // The previewed cell no longer exists once the results are gone.
        get().hideCellPreview(tabId);
        set((state) => {
            const { [tabId]: _, ...restResults } = state.tabQueryResults;
            const { [tabId]: __, ...restIndices } = state.activeResultIndex;
            const { [tabId]: ___, ...restCounters } = state.tabResultCounters;
            const { [tabId]: ____, ...restScrollPositions } = state.resultScrollPosition;
            return {
                tabQueryResults: restResults,
                activeResultIndex: restIndices,
                tabResultCounters: restCounters,
                resultScrollPosition: restScrollPositions
            };
        });
    },

    closeResult: (tabId, resultIndex) => {
        // Result indices shift, so a preview pinned to one of them is no longer
        // trustworthy.
        get().hideCellPreview(tabId);
        set((state) => {
            const currentResults = state.tabQueryResults[tabId] || [];
            const newResults = [...currentResults];
            newResults.splice(resultIndex, 1);

            const currentScrollPositions = state.resultScrollPosition[tabId] || {};
            const newScrollPositions: Record<number, { top: number; left: number }> = {};
            Object.entries(currentScrollPositions).forEach(([k, v]) => {
                const idx = parseInt(k, 10);
                if (idx < resultIndex) {
                    newScrollPositions[idx] = v;
                } else if (idx > resultIndex) {
                    newScrollPositions[idx - 1] = v;
                }
            });

            const { [tabId]: _removedTabScroll, ...restScrollPositions } = state.resultScrollPosition;

            // Update active index if needed
            let newActiveIndex = state.activeResultIndex[tabId] || 0;
            if (newActiveIndex >= newResults.length) {
                newActiveIndex = Math.max(0, newResults.length - 1);
            }

            return {
                tabQueryResults: { ...state.tabQueryResults, [tabId]: newResults },
                activeResultIndex: { ...state.activeResultIndex, [tabId]: newActiveIndex },
                resultScrollPosition: newResults.length > 0
                    ? { ...state.resultScrollPosition, [tabId]: newScrollPositions }
                    : restScrollPositions,
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

    setResultScrollPosition: (tabId, resultIndex, top, left) => {
        set((state) => ({
            resultScrollPosition: {
                ...state.resultScrollPosition,
                [tabId]: {
                    ...(state.resultScrollPosition[tabId] || {}),
                    [resultIndex]: { top, left }
                }
            }
        }));
    },

    getResultScrollPosition: (tabId, resultIndex) => {
        const positions = get().resultScrollPosition[tabId];
        return positions ? positions[resultIndex] : null;
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
            const newScrollPositions = reorderMap(state.resultScrollPosition[tabId]);

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
                resultScrollPosition: {
                    ...state.resultScrollPosition,
                    [tabId]: newScrollPositions
                },
                activeResultIndex: {
                    ...state.activeResultIndex,
                    [tabId]: toIndex // Focus moved result
                }
            };
        });
    },

    setEnableStickyNotes: (enabled) => {
        set({ enableStickyNotes: enabled });
        get().saveAppSettings();
    },
    setMaxResultRows: (rows) => {
        set({ maxResultRows: rows });
        get().saveAppSettings();
    },

    showCellPreview: (tabId, resultIndex, rowIndex, colIndex, value, columnName, dataType, options) => {
        const referenceRequest = options?.referenceRequest ?? null;
        // Keep whichever view this tab was last on, unless the caller asks for
        // one, and fall back to Value when the new cell has no reference.
        const requestedTab = options?.tab;
        const currentTab = get().cellPreviewPanel.byTab[tabId]?.activeTab ?? 'value';
        const desiredTab: CellPreviewTab = requestedTab ?? currentTab;
        const activeTab: CellPreviewTab = desiredTab === 'reference' && !referenceRequest ? 'value' : desiredTab;

        set((state) => ({
            cellPreviewPanel: {
                ...state.cellPreviewPanel,
                byTab: {
                    ...state.cellPreviewPanel.byTab,
                    [tabId]: {
                        activeTab,
                        queryId: options?.queryId ?? null,
                        selectedCell: {
                            tabId,
                            resultIndex,
                            rowIndex,
                            colIndex,
                            value,
                            columnName,
                            dataType,
                            referenceRequest
                        }
                    }
                }
            }
        }));

        if (activeTab === 'reference' && referenceRequest) {
            void get().openReferencePreview(referenceRequest);
        }
    },

    hideCellPreview: (tabId) => {
        const targetTabId = tabId ?? get().activeTabId;
        if (!targetTabId) return;

        const entry = get().cellPreviewPanel.byTab[targetTabId];
        if (!entry) return;

        set((state) => {
            const { [targetTabId]: _removed, ...rest } = state.cellPreviewPanel.byTab;
            return { cellPreviewPanel: { ...state.cellPreviewPanel, byTab: rest } };
        });

        // Only drop the reference data if it belonged to the closed preview.
        if (entry.activeTab === 'reference') {
            get().clearReferencePreview();
        }
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
    },

    setCellPreviewTab: (tabId, tab) => {
        const entry = get().cellPreviewPanel.byTab[tabId];
        if (!entry) return;
        if (tab === 'reference' && !entry.selectedCell.referenceRequest) return;

        set((state) => ({
            cellPreviewPanel: {
                ...state.cellPreviewPanel,
                byTab: {
                    ...state.cellPreviewPanel.byTab,
                    [tabId]: { ...entry, activeTab: tab }
                }
            }
        }));

        if (tab === 'reference' && entry.selectedCell.referenceRequest) {
            void get().openReferencePreview(entry.selectedCell.referenceRequest);
        }
    }
});
