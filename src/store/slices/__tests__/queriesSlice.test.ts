import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createQueriesSlice } from '../queriesSlice';
import { createSpacesSlice } from '../spacesSlice';
import { createTabsSlice } from '../tabsSlice';
import { create } from 'zustand';
import * as api from '../../../api';

// Mocks
vi.mock('../../../api', () => ({
    executeQuery: vi.fn(),
    cancelQuery: vi.fn(),
    touchTab: vi.fn(),
    getSpaces: vi.fn(),
    getTabsBySpace: vi.fn(),
    getAppSettings: vi.fn(),
    updateSpaceLastActiveTab: vi.fn(),
    getFoldersBySpace: vi.fn(),
    getArchivedTabsCount: vi.fn(),
}));

const useTestStore = create<any>((set, get, api) => ({
    ...createSpacesSlice(set, get, api),
    ...createTabsSlice(set, get, api),
    ...createQueriesSlice(set, get, api),
    // Mocks for dependencies
    addToast: vi.fn(),
    loadArchiveCount: vi.fn(),
    loadFolders: vi.fn(),
    touchActiveTab: vi.fn(),
    saveAppSettings: vi.fn(),
    openReferencePreview: vi.fn(),
    clearReferencePreview: vi.fn(),
}));

/** Minimal reference request for preview tests. */
const referenceRequest = (tabId: string) => ({
    tabId,
    reference: {
        constraintName: 'FK_Test',
        sourceSchema: 'dbo',
        sourceTable: 'Transaction',
        targetSchema: 'dbo',
        targetTable: 'TransactionStatus',
        targetColumn: 'Code',
        columnPairs: [{ sourceColumn: 'Status', targetColumn: 'Code' }],
        isVirtual: false,
    },
    filters: [{ targetColumn: 'Code', value: 4, dataType: 'int' }],
    skippedColumns: [],
    value: 4,
}) as any;

describe('queriesSlice', () => {
    beforeEach(() => {
        useTestStore.setState({
            tabs: [],
            activeTabId: null,
            spaces: [],
            activeSpaceId: null,
            tabQueryResults: {},
            tabExecuting: {},
            activeResultIndex: {},
            spaceConnectionStatus: { is_connected: true } as any,
        });
        vi.clearAllMocks();
    });

    it('should execute a query successfully', async () => {
        const spaceId = 'space-1';
        const tabId = 'tab-1';

        useTestStore.setState({
            activeSpaceId: spaceId,
            spaces: [{ id: spaceId, connection_database: 'master' } as any],
            tabs: [{ id: tabId, space_id: spaceId, title: 'Query' } as any],
            activeTabId: tabId
        });

        const mockResults = [{
            query_id: 'q1',
            columns: [{ name: 'id', data_type: 'int' }],
            rows: [[1]],
            row_count: 1,
            execution_time_ms: 10,
            is_complete: true,
            statement_index: 0,
        }];

        (api.executeQuery as any).mockResolvedValue(mockResults);

        await useTestStore.getState().executeQuery(tabId, 'SELECT 1');

        expect(api.executeQuery).toHaveBeenCalledWith(spaceId, 'SELECT 1', 'master', undefined, 5000);
        expect(useTestStore.getState().tabQueryResults[tabId]).toEqual(expect.arrayContaining([
            expect.objectContaining({ rows: [[1]], displayId: 1 })
        ]));
        expect(useTestStore.getState().tabExecuting[tabId]).toBe(false);
        expect(useTestStore.getState().tabResultCounters[tabId]).toBe(1);
    });

    it('should handle query execution error', async () => {
        const spaceId = 'space-1';
        const tabId = 'tab-1';

        useTestStore.setState({
            activeSpaceId: spaceId,
            spaces: [{ id: spaceId } as any],
            tabs: [{ id: tabId } as any],
        });

        const errorMsg = 'Syntax error';
        (api.executeQuery as any).mockRejectedValue(new Error(errorMsg));

        await useTestStore.getState().executeQuery(tabId, 'SELECT * FROM');

        expect(useTestStore.getState().tabQueryResults[tabId]).toBeUndefined(); // Or however error handling sets state
        // The slice implementation catches errors, logs them, and returns null without setting result state.

        expect(useTestStore.getState().tabExecuting[tabId]).toBe(false);
        expect(useTestStore.getState().addToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
    it('should reorder results and their associated metadata', () => {
        const tabId = 'tab-1';
        const results = [
            { query_id: '1', rows: [[1]], displayId: 1 },
            { query_id: '2', rows: [[2]], displayId: 2 },
            { query_id: '3', rows: [[3]], displayId: 3 }
        ] as any[];

        useTestStore.setState({
            tabQueryResults: { [tabId]: results },
            resultCustomNames: {
                [tabId]: {
                    0: 'Result 1',
                    1: 'Result 2',
                    2: 'Result 3'
                }
            },
            resultColumnOrder: {
                [tabId]: {
                    0: [0, 1],
                    1: [1, 0],
                    2: [0, 1]
                }
            },
            resultScrollPosition: {
                [tabId]: {
                    0: { top: 10, left: 20 },
                    1: { top: 30, left: 40 },
                    2: { top: 50, left: 60 }
                }
            }
        });

        // Move item at index 0 to index 2
        // Initial: [1, 2, 3]
        // Target: [2, 3, 1]
        useTestStore.getState().reorderQueryResults(tabId, 0, 2);

        const newResults = useTestStore.getState().tabQueryResults[tabId];
        expect(newResults[0].query_id).toBe('2');
        expect(newResults[1].query_id).toBe('3');
        expect(newResults[2].query_id).toBe('1');

        // Check custom names
        const names = useTestStore.getState().resultCustomNames[tabId];
        expect(names[0]).toBe('Result 2');
        expect(names[1]).toBe('Result 3');
        expect(names[2]).toBe('Result 1');

        // Check displayIds are preserved after reordering
        // Old Index 0 (displayId: 1) -> New Index 2
        // Old Index 1 (displayId: 2) -> New Index 0
        // Old Index 2 (displayId: 3) -> New Index 1
        expect(newResults[0].displayId).toBe(2);
        expect(newResults[1].displayId).toBe(3);
        expect(newResults[2].displayId).toBe(1);

        // Check column order
        // Index 0 should now be [1, 0] (from old index 1)
        // Index 1 should now be [0, 1] (from old index 2)
        // Index 2 should now be [0, 1] (from old index 0)
        const orders = useTestStore.getState().resultColumnOrder[tabId];
        expect(orders[0]).toEqual([1, 0]);
        expect(orders[1]).toEqual([0, 1]);
        expect(orders[2]).toEqual([0, 1]);

        // Check scroll positions reorder with results
        const scroll = useTestStore.getState().resultScrollPosition[tabId];
        expect(scroll[0]).toEqual({ top: 30, left: 40 });
        expect(scroll[1]).toEqual({ top: 50, left: 60 });
        expect(scroll[2]).toEqual({ top: 10, left: 20 });
    });
    it('should replace only the active result when executing a query', async () => {
        const spaceId = 'space-1';
        const tabId = 'tab-1';

        // Initial state with 3 results
        const initialResults = [
            { query_id: 'q1', rows: [['old1']] },
            { query_id: 'q2', rows: [['old2']] },
            { query_id: 'q3', rows: [['old3']] }
        ] as any[];

        useTestStore.setState({
            activeSpaceId: spaceId,
            spaces: [{ id: spaceId, connection_database: 'master' } as any],
            tabs: [{ id: tabId, space_id: spaceId, title: 'Query' } as any],
            activeTabId: tabId,
            tabQueryResults: { [tabId]: initialResults },
            activeResultIndex: { [tabId]: 1 }, // Active is the middle one (index 1)
            resultCustomNames: {
                [tabId]: {
                    0: 'Result 1',
                    1: 'Result 2',
                    2: 'Result 3'
                }
            }
        });

        const newResult = [{
            query_id: 'new_q',
            rows: [['new']]
        }];

        (api.executeQuery as any).mockResolvedValue(newResult);

        await useTestStore.getState().executeQuery(tabId, 'SELECT new');

        const updatedResults = useTestStore.getState().tabQueryResults[tabId];

        // Should still have 3 results
        expect(updatedResults).toHaveLength(3);

        // Index 0 and 2 should be unchanged
        expect(updatedResults[0].query_id).toBe('q1');
        expect(updatedResults[2].query_id).toBe('q3');

        // Index 1 should be the new result
        expect(updatedResults[1].query_id).toBe('new_q');
        expect(updatedResults[1].rows).toEqual([['new']]);

        // Active index should remain 1
        expect(useTestStore.getState().activeResultIndex[tabId]).toBe(1);
    });

    it('should insert multiple results at active index and shift subsequent results', async () => {
        const spaceId = 'space-1';
        const tabId = 'tab-1';

        // Initial state with 3 results
        const initialResults = [
            { query_id: 'q1', rows: [['old1']] },
            { query_id: 'q2', rows: [['old2']] },
            { query_id: 'q3', rows: [['old3']] }
        ] as any[];

        useTestStore.setState({
            activeSpaceId: spaceId,
            spaces: [{ id: spaceId, connection_database: 'master' } as any],
            tabs: [{ id: tabId, space_id: spaceId, title: 'Query' } as any],
            activeTabId: tabId,
            tabQueryResults: { [tabId]: initialResults },
            activeResultIndex: { [tabId]: 1 }, // Active is index 1
            resultCustomNames: {
                [tabId]: {
                    0: 'Name 0',
                    1: 'Name 1',
                    2: 'Name 2'
                }
            }
        });

        // execution returns 2 results
        const newResults = [
            { query_id: 'new_q1', rows: [['new1']] },
            { query_id: 'new_q2', rows: [['new2']] }
        ];

        (api.executeQuery as any).mockResolvedValue(newResults);

        await useTestStore.getState().executeQuery(tabId, 'SELECT multiple');

        const updatedResults = useTestStore.getState().tabQueryResults[tabId];

        // Should have 3 - 1 + 2 = 4 results
        expect(updatedResults).toHaveLength(4);

        // Index 0: unchanged
        expect(updatedResults[0].query_id).toBe('q1');

        // Index 1, 2: new results
        expect(updatedResults[1].query_id).toBe('new_q1');
        expect(updatedResults[2].query_id).toBe('new_q2');

        // Index 3: shifted old index 2
        expect(updatedResults[3].query_id).toBe('q3');

        // Check metadata shifting
        const names = useTestStore.getState().resultCustomNames[tabId];
        expect(names[0]).toBe('Name 0'); // Unchanged
        // Name 1 was at index 1, replaced by new result, so it's gone/reset for the new result at index 1
        // Name 2 was at index 2, should be shifted to index 3
        expect(names[3]).toBe('Name 2');

        // Active index remains 1 (start of new results)
        expect(useTestStore.getState().activeResultIndex[tabId]).toBe(1);
    });

    it('should shift and remove scroll positions when closing a result', () => {
        const tabId = 'tab-1';

        useTestStore.setState({
            tabQueryResults: {
                [tabId]: [
                    { query_id: 'q1', rows: [[1]] },
                    { query_id: 'q2', rows: [[2]] },
                    { query_id: 'q3', rows: [[3]] }
                ]
            },
            activeResultIndex: { [tabId]: 2 },
            resultScrollPosition: {
                [tabId]: {
                    0: { top: 5, left: 6 },
                    1: { top: 15, left: 16 },
                    2: { top: 25, left: 26 }
                }
            }
        });

        // Remove middle result index 1
        useTestStore.getState().closeResult(tabId, 1);

        const updatedResults = useTestStore.getState().tabQueryResults[tabId];
        expect(updatedResults).toHaveLength(2);
        expect(updatedResults[0].query_id).toBe('q1');
        expect(updatedResults[1].query_id).toBe('q3');

        const scroll = useTestStore.getState().resultScrollPosition[tabId];
        expect(scroll[0]).toEqual({ top: 5, left: 6 });
        expect(scroll[1]).toEqual({ top: 25, left: 26 });
        expect(scroll[2]).toBeUndefined();
    });

    it('should remove scroll positions when clearing results', () => {
        const tabId = 'tab-1';

        useTestStore.setState({
            tabQueryResults: {
                [tabId]: [{ query_id: 'q1', rows: [[1]] }]
            },
            resultScrollPosition: {
                [tabId]: {
                    0: { top: 12, left: 24 }
                }
            }
        });

        useTestStore.getState().clearQueryResult(tabId);

        expect(useTestStore.getState().tabQueryResults[tabId]).toBeUndefined();
        expect(useTestStore.getState().resultScrollPosition[tabId]).toBeUndefined();
    });

    describe('cell preview (per tab)', () => {
        beforeEach(() => {
            useTestStore.setState({
                cellPreviewPanel: { width: 500, formatterType: 'auto', byTab: {} },
            });
        });

        const show = (tabId: string, options?: Record<string, unknown>) =>
            useTestStore
                .getState()
                .showCellPreview(tabId, 0, 3, 2, 'value', 'Status', 'int', options);

        it('remembers a preview per tab so switching tabs keeps both', () => {
            show('tab-1');
            show('tab-2', { queryId: 'q2' });

            const { byTab } = useTestStore.getState().cellPreviewPanel;
            expect(Object.keys(byTab).sort()).toEqual(['tab-1', 'tab-2']);
            expect(byTab['tab-1'].selectedCell.tabId).toBe('tab-1');
            expect(byTab['tab-2'].queryId).toBe('q2');
        });

        it('closes only the requested tab, leaving other tabs untouched', () => {
            show('tab-1');
            show('tab-2');

            useTestStore.getState().hideCellPreview('tab-1');

            const { byTab } = useTestStore.getState().cellPreviewPanel;
            expect(byTab['tab-1']).toBeUndefined();
            expect(byTab['tab-2']).toBeDefined();
        });

        it('drops reference data only when the closed preview was showing it', () => {
            show('tab-1', { referenceRequest: referenceRequest('tab-1'), tab: 'reference' });
            show('tab-2');

            useTestStore.getState().hideCellPreview('tab-2');
            expect(useTestStore.getState().clearReferencePreview).not.toHaveBeenCalled();

            useTestStore.getState().hideCellPreview('tab-1');
            expect(useTestStore.getState().clearReferencePreview).toHaveBeenCalled();
        });

        it('keeps the tab on its previous view and falls back when there is no reference', () => {
            show('tab-1', { referenceRequest: referenceRequest('tab-1'), tab: 'reference' });
            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1'].activeTab).toBe('reference');

            // Another foreign key cell in the same tab stays on the reference view
            show('tab-1', { referenceRequest: referenceRequest('tab-1') });
            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1'].activeTab).toBe('reference');

            // A cell with no reference falls back to the value view
            show('tab-1');
            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1'].activeTab).toBe('value');
        });

        it('refuses to switch to the reference view without a reference', () => {
            show('tab-1');
            useTestStore.getState().setCellPreviewTab('tab-1', 'reference');
            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1'].activeTab).toBe('value');

            show('tab-1', { referenceRequest: referenceRequest('tab-1') });
            useTestStore.getState().setCellPreviewTab('tab-1', 'reference');
            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1'].activeTab).toBe('reference');
            expect(useTestStore.getState().openReferencePreview).toHaveBeenCalled();
        });

        it('closes the preview when the tab results are cleared', () => {
            show('tab-1');
            useTestStore.setState({ tabQueryResults: { 'tab-1': [{ query_id: 'q1', rows: [[1]] }] } as any });

            useTestStore.getState().clearQueryResult('tab-1');

            expect(useTestStore.getState().cellPreviewPanel.byTab['tab-1']).toBeUndefined();
        });
    });
});
