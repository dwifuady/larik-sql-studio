import { StateCreator } from 'zustand';
import type { QueryResult, TableInfo } from '../../types';
import * as api from '../../api';
import { generateCreateTableScript } from '../../utils/sql';
import type { AppState } from '../index';

/** What the peek popup is currently showing. */
export type PeekKind = 'table-data' | 'object-source';

export interface PeekState {
    open: boolean;
    kind: PeekKind | null;
    /** Object type that was peeked, for icon/label purposes. */
    objectType: 'table' | 'view' | 'routine' | null;
    schema: string;
    name: string;
    database: string | null;
    /** The SQL used for a data peek (also seeds the tab on expand). */
    query: string | null;
    /** Result rows for a data peek. */
    result: QueryResult | null;
    /** Source text for an object-source peek. */
    content: string | null;
    loading: boolean;
    error: string | null;
}

export interface PeekSlice {
    peek: PeekState;
    openTablePeek: (table: TableInfo) => Promise<void>;
    openObjectPeek: (
        objectType: 'view' | 'routine',
        schemaName: string,
        objectName: string,
    ) => Promise<void>;
    openTableSourcePeek: (table: TableInfo) => void;
    closePeek: () => void;
    expandPeek: () => Promise<void>;
}

const initialPeek: PeekState = {
    open: false,
    kind: null,
    objectType: null,
    schema: '',
    name: '',
    database: null,
    query: null,
    result: null,
    content: null,
    loading: false,
    error: null,
};

// Monotonic token so a slow peek request can't overwrite a newer one when the
// user clicks another object before the first query returns.
let peekToken = 0;

export const createPeekSlice: StateCreator<AppState, [], [], PeekSlice> = (set, get) => ({
    peek: initialPeek,

    openTablePeek: async (table) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            get().addToast({ type: 'error', message: 'No active space' });
            return;
        }

        const database = get().schemaInfo?.database_name ?? null;
        const query = `SELECT TOP 100 * FROM [${table.schema_name}].[${table.table_name}]`;
        const token = ++peekToken;

        set({
            peek: {
                ...initialPeek,
                open: true,
                kind: 'table-data',
                objectType: table.table_type === 'VIEW' ? 'view' : 'table',
                schema: table.schema_name,
                name: table.table_name,
                database,
                query,
                loading: true,
            },
        });

        try {
            const results = await api.executeQuery(spaceId, query, database);
            if (token !== peekToken) return; // superseded by a newer peek
            const result = results[0] ?? null;
            set((state) => ({
                peek: {
                    ...state.peek,
                    loading: false,
                    result,
                    error: result?.error ?? null,
                },
            }));
        } catch (error) {
            if (token !== peekToken) return;
            set((state) => ({
                peek: {
                    ...state.peek,
                    loading: false,
                    error: error instanceof Error ? error.message : String(error),
                },
            }));
        }
    },

    openObjectPeek: async (objectType, schemaName, objectName) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            get().addToast({ type: 'error', message: 'No active space' });
            return;
        }

        const database = get().schemaInfo?.database_name ?? null;
        // OBJECT_DEFINITION returns the full CREATE text of a view/proc/function.
        const query = `SELECT OBJECT_DEFINITION(OBJECT_ID(N'[${schemaName}].[${objectName}]')) AS [definition]`;
        const token = ++peekToken;

        set({
            peek: {
                ...initialPeek,
                open: true,
                kind: 'object-source',
                objectType,
                schema: schemaName,
                name: objectName,
                database,
                loading: true,
            },
        });

        try {
            const results = await api.executeQuery(spaceId, query, database);
            if (token !== peekToken) return;
            const cell = results[0]?.rows?.[0]?.[0];
            const definition =
                typeof cell === 'string' && cell.trim().length > 0
                    ? cell
                    : '-- No definition available (the object may be encrypted or was not found).';
            set((state) => ({
                peek: {
                    ...state.peek,
                    loading: false,
                    content: definition,
                    error: results[0]?.error ?? null,
                },
            }));
        } catch (error) {
            if (token !== peekToken) return;
            set((state) => ({
                peek: {
                    ...state.peek,
                    loading: false,
                    error: error instanceof Error ? error.message : String(error),
                },
            }));
        }
    },

    openTableSourcePeek: (table) => {
        // OBJECT_DEFINITION is NULL for base tables, so we synthesize the
        // CREATE TABLE script from schema metadata we already hold in memory.
        peekToken++; // invalidate any in-flight data/source request
        set({
            peek: {
                ...initialPeek,
                open: true,
                kind: 'object-source',
                objectType: 'table',
                schema: table.schema_name,
                name: table.table_name,
                database: get().schemaInfo?.database_name ?? null,
                content: generateCreateTableScript(table),
                loading: false,
            },
        });
    },

    closePeek: () => {
        // Invalidate any in-flight request so its result is discarded.
        peekToken++;
        set({ peek: initialPeek });
    },

    expandPeek: async () => {
        const { peek } = get();
        if (!peek.open) return;

        if (peek.kind === 'table-data' && peek.query) {
            const tab = await get().createTab(peek.name, 'query', peek.query, peek.database);
            get().closePeek();
            if (tab) {
                // Re-run so the expanded tab shows the same rows as the peek.
                await get().executeQuery(tab.id, peek.query);
            }
        } else if (peek.kind === 'object-source' && peek.content !== null) {
            await get().createTab(peek.name, 'query', peek.content, peek.database);
            get().closePeek();
        }
    },
});
