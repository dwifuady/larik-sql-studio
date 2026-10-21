import { describe, it, expect, vi, beforeEach } from 'vitest';
import { create } from 'zustand';
import { createSpacesSlice } from '../spacesSlice';
import { createTabsSlice } from '../tabsSlice';
import { createSchemaSlice } from '../schemaSlice';
import * as api from '../../../api';
import type { SchemaInfo } from '../../../types';

vi.mock('../../../api', () => ({
    getSchemaInfo: vi.fn(),
    refreshSchema: vi.fn(),
    updateTabDatabase: vi.fn(),
    updateSpaceLastActiveTab: vi.fn(),
    createTab: vi.fn(),
    getFoldersBySpace: vi.fn(),
    touchTab: vi.fn(),
}));

const useTestStore = create<any>((set, get, storeApi) => ({
    ...createSpacesSlice(set, get, storeApi),
    ...createTabsSlice(set, get, storeApi),
    ...createSchemaSlice(set, get, storeApi),
    // Dependencies provided by other slices in the real store
    spaceDatabases: [],
    loadVirtualReferences: vi.fn(),
    isConnected: () => true,
}));

const makeSpace = (id: string, database: string | null) => ({
    id,
    name: `Space ${id}`,
    color: '#0078d4',
    sort_order: 0,
    connection_database: database,
    connection_driver: 'postgres',
    driver: 'postgres',
    created_at: '',
    updated_at: '',
});

const makeSchema = (database: string): SchemaInfo => ({
    database_name: database,
    schemas: ['dbo'],
    tables: [{
        table_name: `${database}_t`, schema_name: 'dbo', table_type: 'TABLE',
        columns: [], is_view: false,
    } as any],
    relationships: [],
    routines: [],
    fetched_at: '',
});

/** Two tabs in one space with different databases; tab1 (dbA) is active. */
const seed = (dbA: string, dbB: string) => {
    useTestStore.setState({
        activeSpaceId: 's1',
        spaces: [makeSpace('s1', 'dbA')],
        tabs: [
            { id: 't1', space_id: 's1', title: 'A', database: dbA, tab_type: 'query' },
            { id: 't2', space_id: 's1', title: 'B', database: dbB, tab_type: 'query' },
        ],
        activeTabId: 't1',
        spaceDatabases: [{ name: dbA, hasAccess: true }, { name: dbB, hasAccess: true }],
    });
};

describe('schemaSlice (cross-database autocomplete regression)', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        useTestStore.setState({
            schemaInfo: null,
            schemaDatabase: null,
            schemaError: null,
            schemaLoading: false,
        });
    });

    it('does not publish a stale load when the tab switches databases mid-flight', async () => {
        seed('dbA', 'dbB');
        const resolvers: Record<string, () => void> = {};
        vi.mocked(api.getSchemaInfo).mockImplementation(((
            _connectionId: string,
            database: string
        ) => new Promise((resolve) => {
            resolvers[database] = () => resolve(makeSchema(database));
        })) as never);

        // Slow load for tab1/dbA, then user switches to tab2 and its load runs.
        const slow = useTestStore.getState().loadSchemaForTab('t1', 'dbA');
        useTestStore.setState({ activeTabId: 't2' });
        const fast = useTestStore.getState().loadSchemaForTab('t2', 'dbB');
        resolvers['dbB']();
        await fast;
        expect(useTestStore.getState().schemaInfo?.database_name).toBe('dbB');

        // The stale dbA fetch resolves last and must NOT publish.
        resolvers['dbA']();
        await slow;
        expect(useTestStore.getState().schemaInfo?.database_name).toBe('dbB');
        expect(useTestStore.getState().schemaInfo?.tables[0].table_name).toBe('dbB_t');
    });

    it('clears the schema on failure instead of keeping the old database tables', async () => {
        seed('dbA', 'dbB');
        vi.mocked(api.getSchemaInfo).mockResolvedValue(makeSchema('dbA'));
        await useTestStore.getState().loadSchemaForTab('t1', 'dbA');
        expect(useTestStore.getState().schemaInfo?.database_name).toBe('dbA');

        // Switch to t2; its fetch fails and must not leave dbA's tables behind.
        useTestStore.setState({ activeTabId: 't2' });
        vi.mocked(api.getSchemaInfo).mockRejectedValueOnce(new Error('boom'));
        await useTestStore.getState().loadSchemaForTab('t2', 'dbB');

        const state = useTestStore.getState();
        expect(state.schemaInfo).toBeNull();
        expect(state.schemaDatabase).toBeNull();
        expect(state.schemaError).toBe('boom');
    });

    it('drops a foreign published slot immediately when a new load starts', async () => {
        seed('dbA', 'dbB');
        vi.mocked(api.getSchemaInfo).mockResolvedValue(makeSchema('dbA'));
        await useTestStore.getState().loadSchemaForTab('t1', 'dbA');
        expect(useTestStore.getState().schemaInfo).not.toBeNull();

        // A switched-away tab or connectivity drop: new load for dbB and dbA
        // fetch hangs forever. The dbA slot must already be gone.
        vi.mocked(api.getSchemaInfo).mockImplementation(() => new Promise(() => { }) as never);
        useTestStore.setState({ activeTabId: 't2' });
        void useTestStore.getState().loadSchemaForTab('t2', 'dbB');
        expect(useTestStore.getState().schemaInfo).toBeNull();
        expect(useTestStore.getState().schemaDatabase).toBe('dbB');
        expect(useTestStore.getState().schemaError).toBeNull();
    });

    it('keeps a matching published schema when loadSchema has no explicit database', async () => {
        seed('dbA', 'dbB');
        vi.mocked(api.getSchemaInfo).mockResolvedValue(makeSchema('dbA'));
        await useTestStore.getState().loadSchema();
        expect(useTestStore.getState().schemaDatabase).toBe('dbA');

        // No-op: same target → no unnecessary churn of the published slot.
        expect(useTestStore.getState().schemaInfo?.database_name).toBe('dbA');
    });
});
