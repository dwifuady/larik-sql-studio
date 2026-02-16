import { StateCreator } from 'zustand';
import type { SchemaInfo } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface SchemaSlice {
    schemaInfo: SchemaInfo | null;
    schemaLoading: boolean;
    schemaError: string | null;

    loadSchema: (database?: string) => Promise<void>;
    refreshSchema: () => Promise<void>;
    clearSchema: () => void;
}

export const createSchemaSlice: StateCreator<AppState, [], [], SchemaSlice> = (set, get) => ({
    schemaInfo: null,
    schemaLoading: false,
    schemaError: null,

    loadSchema: async (database) => {
        // If database is provided, we use it, otherwise we check if a database is selected in active tab or space
        // The backend logic for `getSchemaInfo` likely handles current database context if none provided
        // But frontend should probably be explicit

        set({ schemaLoading: true, schemaError: null });
        try {
            const activeTab = get().getActiveTab();
            const activeSpace = get().getActiveSpace();

            const targetDb = database || activeTab?.database || activeSpace?.connection_database;

            if (!targetDb && get().isConnected()) {
                // If connected but no DB selected, maybe just fetch default?
                // api.getSchemaInfo handles `None` as default DB
            }

            const spaceId = get().activeSpaceId;
            if (!spaceId) {
                set({ schemaInfo: null, schemaError: 'No active space' });
                return;
            }

            const schema = await api.getSchemaInfo(spaceId, targetDb || 'master');
            set({ schemaInfo: schema, schemaLoading: false });
        } catch (error) {
            console.error('Failed to load schema:', error);
            set({
                schemaLoading: false,
                schemaError: error instanceof Error ? error.message : String(error)
            });
        }
    },

    refreshSchema: async () => {
        const spaceId = get().activeSpaceId;
        const activeTab = get().getActiveTab();
        const activeSpace = get().getActiveSpace();
        const dbName = activeTab?.database || activeSpace?.connection_database;

        if (!spaceId || !dbName) return;

        set({ schemaLoading: true, schemaError: null });
        try {
            await api.refreshSchema(spaceId, dbName);
            // Reload
            const schema = await api.getSchemaInfo(spaceId, dbName);
            set({ schemaInfo: schema, schemaLoading: false });
        } catch (error) {
            console.error('Failed to refresh schema:', error);
            set({
                schemaLoading: false,
                schemaError: error instanceof Error ? error.message : String(error)
            });
        }
    },

    clearSchema: () => {
        set({ schemaInfo: null, schemaError: null });
    }
});
