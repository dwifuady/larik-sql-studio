import { StateCreator } from 'zustand';
import type { SchemaInfo } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';
import { defaultDatabaseFor } from '../../types';

export interface SchemaSlice {
    schemaInfo: SchemaInfo | null;
    schemaLoading: boolean;
    schemaError: string | null;
    /** Database the published schemaInfo belongs to; null when none/stale. */
    schemaDatabase: string | null;
    expandedNodes: Set<string>; // Persist expansion state

    /** Load schema for the active tab's database (or an explicit database). */
    loadSchema: (database?: string) => Promise<void>;
    /**
     * Load schema bound to a specific tab. The result is published only while
     * that tab is still active — switching tabs on a slow connection can no
     * longer leave a foreign database's tables in the autocomplete.
     */
    loadSchemaForTab: (tabId: string, database: string) => Promise<void>;
    refreshSchema: () => Promise<void>;
    /** Drop schema that belongs to the given database (or all of it). */
    clearSchema: (database?: string | null) => void;
    toggleNodeExpansion: (nodeId: string) => void;
    expandNode: (nodeId: string) => void;
    collapseNode: (nodeId: string) => void;
}

/**
 * Monotonic token handed to each in-flight schema fetch. When a load resolves,
 * it may only publish if its token is still the newest — otherwise a slower
 * request for an older database would overwrite a newer one (last writer
 * wins → wrong database's tables in the editor).
 */
let schemaLoadToken = 0;

/** Whether the published schemaInfo is exactly the schema for `db`. */
const schemaMatchesDb = (info: SchemaInfo | null, db: string | null): boolean =>
    !!info && !!db && info.database_name === db;

export const createSchemaSlice: StateCreator<AppState, [], [], SchemaSlice> = (set, get) => ({
    schemaInfo: null,
    schemaLoading: false,
    schemaError: null,
    schemaDatabase: null,
    expandedNodes: new Set(),

    toggleNodeExpansion: (nodeId) => {
        set((state) => {
            const newExpanded = new Set(state.expandedNodes);
            if (newExpanded.has(nodeId)) {
                newExpanded.delete(nodeId);
            } else {
                newExpanded.add(nodeId);
            }
            return { expandedNodes: newExpanded };
        });
    },

    expandNode: (nodeId) => {
        set((state) => {
            const newExpanded = new Set(state.expandedNodes);
            newExpanded.add(nodeId);
            return { expandedNodes: newExpanded };
        });
    },

    collapseNode: (nodeId) => {
        set((state) => {
            const newExpanded = new Set(state.expandedNodes);
            newExpanded.delete(nodeId);
            return { expandedNodes: newExpanded };
        });
    },

    loadSchema: async (database) => {
        const spaceId = get().activeSpaceId;

        // If no space, we can't load schema
        if (!spaceId) {
            set({ schemaInfo: null, schemaDatabase: null, schemaError: null });
            return;
        }

        // Determine target database
        // 1. Explicit argument
        // 2. Active tab's selected database
        // 3. Active space's default database
        const activeTab = get().getActiveTab();
        const activeSpace = get().getActiveSpace();

        const targetDb = database || activeTab?.database || defaultDatabaseFor(activeSpace, get().spaceDatabases);

        if (!targetDb) {
            // No database context available to load schema
            set({ schemaInfo: null, schemaDatabase: null });
            return;
        }

        // Don't fetch if not connected (unless we implement offline schema later)
        if (!get().isConnected()) {
            set({ schemaInfo: null, schemaDatabase: null, schemaError: 'Not connected' });
            return;
        }

        // Publish the target immediately so database A's tables can never be
        // served to a reader waiting on database B, even while loading.
        set((state) => {
            const keep = schemaMatchesDb(state.schemaInfo, targetDb);
            return {
                schemaInfo: keep ? state.schemaInfo : null,
                schemaDatabase: targetDb,
                schemaLoading: true,
                schemaError: null,
            };
        });

        const token = ++schemaLoadToken;
        try {
            const schema = await api.getSchemaInfo(spaceId, targetDb);
            if (token !== schemaLoadToken || get().schemaDatabase !== targetDb) {
                return; // Superseded by a newer load for a different database
            }
            set({ schemaInfo: schema, schemaLoading: false });
            // User-defined references are scoped to the same space + database.
            void get().loadVirtualReferences(targetDb);
        } catch (error) {
            console.error('Failed to load schema:', error);
            if (token !== schemaLoadToken || get().schemaDatabase !== targetDb) {
                return; // A newer load owns the slot now
            }
            set({
                schemaLoading: false,
                // Clearing avoids serving the previous database's tables to
                // the new database's editor on an invisible failure.
                schemaInfo: null,
                schemaDatabase: null,
                schemaError: error instanceof Error ? error.message : String(error)
            });
        }
    },

    loadSchemaForTab: async (tabId, database) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId || !get().isConnected()) {
            return;
        }

        set((state) => {
            const keep = schemaMatchesDb(state.schemaInfo, database);
            return {
                schemaInfo: keep ? state.schemaInfo : null,
                schemaDatabase: database,
                schemaLoading: true,
                schemaError: null,
            };
        });

        const token = ++schemaLoadToken;
        try {
            const schema = await api.getSchemaInfo(spaceId, database);
            // Publish only while this exact tab is still active — the user may
            // have moved to another tab (or another database) meanwhile.
            const activeTab = get().getActiveTab();
            if (token !== schemaLoadToken || get().schemaDatabase !== database || activeTab?.id !== tabId) {
                return;
            }
            set({ schemaInfo: schema, schemaLoading: false });
            void get().loadVirtualReferences(database);
        } catch (error) {
            console.error('Failed to load schema:', error);
            const activeTab = get().getActiveTab();
            if (token !== schemaLoadToken || get().schemaDatabase !== database || activeTab?.id !== tabId) {
                return;
            }
            set({
                schemaLoading: false,
                schemaInfo: null,
                schemaDatabase: null,
                schemaError: error instanceof Error ? error.message : String(error)
            });
        }
    },

    refreshSchema: async () => {
        const spaceId = get().activeSpaceId;
        const activeTab = get().getActiveTab();
        const activeSpace = get().getActiveSpace();

        // Use current schema's database if available, otherwise fall back to context
        const currentSchemaDb = get().schemaInfo?.database_name;
        const targetDb = currentSchemaDb || activeTab?.database || defaultDatabaseFor(activeSpace, get().spaceDatabases);

        if (!spaceId || !targetDb) return;

        // Force refresh in backend
        await api.refreshSchema(spaceId, targetDb);
        await get().loadSchema(targetDb);
    },

    clearSchema: (database) => {
        set((state) => {
            // Explicit database: drop only a foreign schema (e.g. right before
            // switching a tab to another database); keep a matching one.
            if (database != null && schemaMatchesDb(state.schemaInfo, database)) {
                return {};
            }
            return { schemaInfo: null, schemaDatabase: null, schemaError: null };
        });
    }
});
