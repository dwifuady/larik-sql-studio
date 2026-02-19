import { StateCreator } from 'zustand';
import type { SchemaInfo } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface SchemaSlice {
    schemaInfo: SchemaInfo | null;
    schemaLoading: boolean;
    schemaError: string | null;
    expandedNodes: Set<string>; // Persist expansion state

    loadSchema: (database?: string) => Promise<void>;
    refreshSchema: () => Promise<void>;
    clearSchema: () => void;
    toggleNodeExpansion: (nodeId: string) => void;
    expandNode: (nodeId: string) => void;
    collapseNode: (nodeId: string) => void;
}

export const createSchemaSlice: StateCreator<AppState, [], [], SchemaSlice> = (set, get) => ({
    schemaInfo: null,
    schemaLoading: false,
    schemaError: null,
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
            set({ schemaInfo: null, schemaError: null });
            return;
        }

        // Determine target database
        // 1. Explicit argument
        // 2. Active tab's selected database
        // 3. Active space's default database
        const activeTab = get().getActiveTab();
        const activeSpace = get().getActiveSpace();

        const targetDb = database || activeTab?.database || activeSpace?.connection_database;

        if (!targetDb) {
            // connected but no database selected? 
            // We can try fetching for "master" or let backend handle it, 
            // but usually we want to show schema for the CURRENT context.
            set({ schemaInfo: null });
            return;
        }

        // Don't fetch if not connected (unless we implement offline schema later)
        if (!get().isConnected()) {
            set({ schemaInfo: null, schemaError: 'Not connected' });
            return;
        }

        set({ schemaLoading: true, schemaError: null });
        try {
            const schema = await api.getSchemaInfo(spaceId, targetDb);
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

        // Use current schema's database if available, otherwise fall back to context
        const currentSchemaDb = get().schemaInfo?.database_name;
        const targetDb = currentSchemaDb || activeTab?.database || activeSpace?.connection_database;

        if (!spaceId || !targetDb) return;

        set({ schemaLoading: true, schemaError: null });
        try {
            // Force refresh in backend
            await api.refreshSchema(spaceId, targetDb);
            // Fetch fresh data
            const schema = await api.getSchemaInfo(spaceId, targetDb);
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
