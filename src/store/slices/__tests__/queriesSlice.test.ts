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
}));

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

        expect(api.executeQuery).toHaveBeenCalledWith(spaceId, 'SELECT 1', 'master', undefined);
        expect(useTestStore.getState().tabQueryResults[tabId]).toEqual(expect.arrayContaining([
            expect.objectContaining({ rows: [[1]] })
        ]));
        expect(useTestStore.getState().tabExecuting[tabId]).toBe(false);
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
        // Actually our slice implementation sets a toast on error and returns null, 
        // but does NOT set result state if completely failed (it catches and toasts)
        // Wait, let's check implementation:
        // It catches, logs, toasts, and returns null. executed state set to false.

        expect(useTestStore.getState().tabExecuting[tabId]).toBe(false);
        expect(useTestStore.getState().addToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
    it('should reorder results and their associated metadata', () => {
        const tabId = 'tab-1';
        const results = [
            { query_id: '1', rows: [[1]] },
            { query_id: '2', rows: [[2]] },
            { query_id: '3', rows: [[3]] }
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
        // Index 0 should now be "Result 2"
        // Index 1 should now be "Result 3"
        // Index 2 should now be "Result 1"
        const names = useTestStore.getState().resultCustomNames[tabId];
        expect(names[0]).toBe('Result 2');
        expect(names[1]).toBe('Result 3');
        expect(names[2]).toBe('Result 1');

        // Check column order
        // Index 0 should now be [1, 0] (from old index 1)
        // Index 1 should now be [0, 1] (from old index 2)
        // Index 2 should now be [0, 1] (from old index 0)
        const orders = useTestStore.getState().resultColumnOrder[tabId];
        expect(orders[0]).toEqual([1, 0]);
        expect(orders[1]).toEqual([0, 1]);
        expect(orders[2]).toEqual([0, 1]);
    });
});
