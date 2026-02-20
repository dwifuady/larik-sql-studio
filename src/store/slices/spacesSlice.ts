import { StateCreator } from 'zustand';
import type { Space, CreateSpaceInput, UpdateSpaceInput, ConnectionInfo } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface SpacesSlice {
    spaces: Space[];
    activeSpaceId: string | null;
    spacesLoading: boolean;
    spaceConnectionStatus: ConnectionInfo | null;
    isConnecting: boolean;
    connectionError: string | null;
    spaceDatabases: { name: string; hasAccess: boolean }[];
    databasesLoading: boolean;

    loadSpaces: () => Promise<void>;
    createSpace: (input: CreateSpaceInput) => Promise<Space>;
    updateSpace: (id: string, input: UpdateSpaceInput) => Promise<void>;
    deleteSpace: (id: string) => Promise<void>;
    setActiveSpace: (id: string | null) => Promise<void>;
    reorderSpaces: (spaceIds: string[]) => Promise<void>;
    connectToSpace: () => Promise<boolean>;
    disconnectFromSpace: () => Promise<boolean>;
    refreshSpaceConnectionStatus: () => Promise<void>;
    loadSpaceDatabases: () => Promise<void>;
    testConnection: (
        host: string,
        port: number,
        database: string,
        username: string,
        password: string,
        trustCertificate?: boolean,
        encrypt?: boolean
    ) => Promise<boolean>;

    getActiveSpace: () => Space | null;
    isConnected: () => boolean;
}

export const createSpacesSlice: StateCreator<AppState, [], [], SpacesSlice> = (set, get) => ({
    spaces: [],
    activeSpaceId: null,
    spacesLoading: false,
    spaceConnectionStatus: null,
    isConnecting: false,
    connectionError: null,
    spaceDatabases: [],
    databasesLoading: false,

    getActiveSpace: () => {
        const { spaces, activeSpaceId } = get();
        return spaces.find(s => s.id === activeSpaceId) ?? null;
    },

    isConnected: () => {
        return get().spaceConnectionStatus?.is_connected ?? false;
    },

    loadSpaces: async () => {
        set({ spacesLoading: true });
        try {
            const spaces = await api.getSpaces();
            set({ spaces, spacesLoading: false });

            // Load app settings to restore last opened workspace
            const settings = await api.getAppSettings();

            // Try to restore last opened space if it exists
            if (spaces.length > 0 && !get().activeSpaceId) {
                const lastSpaceId = settings.last_space_id;
                const spaceExists = lastSpaceId && spaces.some(s => s.id === lastSpaceId);

                if (spaceExists) {
                    // Restore last opened space
                    await get().setActiveSpace(lastSpaceId);
                    // Try to restore last opened tab after tabs are loaded
                    if (settings.last_tab_id) {
                        const tabs = get().tabs;
                        const tabExists = tabs.some(t => t.id === settings.last_tab_id);
                        if (tabExists) {
                            set({ activeTabId: settings.last_tab_id });
                        }
                    }
                } else {
                    // Fallback to first space if last space doesn't exist
                    await get().setActiveSpace(spaces[0].id);
                }
            }
        } catch (error) {
            console.error('Failed to load spaces:', error);
            set({ spacesLoading: false });
        }
    },

    createSpace: async (input) => {
        const space = await api.createSpace(input);
        set((state) => ({ spaces: [...state.spaces, space] }));
        return space;
    },

    updateSpace: async (id, input) => {
        const updated = await api.updateSpace(id, input);
        if (updated) {
            set((state) => ({
                spaces: state.spaces.map((s) => (s.id === id ? updated : s)),
            }));
            // Refresh connection status if updating active space
            if (id === get().activeSpaceId) {
                await get().refreshSpaceConnectionStatus();
            }
        }
    },

    deleteSpace: async (id) => {
        await api.deleteSpace(id);
        set((state) => {
            const newSpaces = state.spaces.filter((s) => s.id !== id);
            const newActiveSpaceId = state.activeSpaceId === id
                ? (newSpaces[0]?.id ?? null)
                : state.activeSpaceId;
            return { spaces: newSpaces, activeSpaceId: newActiveSpaceId };
        });

        // Load tabs for new active space
        const newActiveSpaceId = get().activeSpaceId;
        if (newActiveSpaceId) {
            await get().loadTabs(newActiveSpaceId);
            await get().refreshSpaceConnectionStatus();
        } else {
            set({
                tabs: [],
                activeTabId: null,
                spaceConnectionStatus: null
            });
        }
    },

    setActiveSpace: async (id) => {
        // Keep connection alive when switching spaces - don't disconnect
        // But clear space-specific state IMMEDIATELY to avoid stale data triggering effects

        set({
            activeSpaceId: id,
            activeTabId: null,
            connectionError: null,
            // Clear ALL space-specific state immediately to prevent race conditions
            // The QueryEditor's useEffect checks isConnected, so we must clear this first
            spaceConnectionStatus: null,
            spaceDatabases: [],
            schemaInfo: null,
            schemaError: null,
        });

        if (id) {
            await get().loadTabs(id);
            await get().loadFolders(id);
            await get().refreshSpaceConnectionStatus();
            // If the new space is connected, reload databases and schema
            if (get().isConnected()) {
                await get().loadSpaceDatabases();
                await get().loadSchema();
            }
            // Persist last opened space
            await get().saveAppSettings();
        } else {
            set({ tabs: [], folders: [] });
            // Persist null space
            await get().saveAppSettings();
        }
    },

    reorderSpaces: async (spaceIds) => {
        await api.reorderSpaces(spaceIds);
        await get().loadSpaces();
    },

    // Connection Actions
    connectToSpace: async () => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) return false;

        set({ isConnecting: true, connectionError: null });
        try {
            const success = await api.connectToSpace(spaceId);
            if (success) {
                await get().refreshSpaceConnectionStatus();
                await get().loadSpaceDatabases();
                await get().loadSchema();
            } else {
                set({ connectionError: 'Failed to connect (unknown error)' });
            }
            set({ isConnecting: false });
            return success;
        } catch (error) {
            console.error('Connection failed:', error);
            set({
                isConnecting: false,
                connectionError: error instanceof Error ? error.message : String(error)
            });

            // Check for password expired error
            const errorMessage = error instanceof Error ? error.message : String(error);
            if (errorMessage.includes("Password expired")) {
                set({
                    connectionError: "Your password has expired. Please change it using SQL Server Management Studio or another tool, then try again."
                });
            }

            return false;
        }
    },

    disconnectFromSpace: async () => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) return false;

        try {
            const success = await api.disconnectFromSpace(spaceId);
            if (success) {
                set({
                    spaceConnectionStatus: null,
                    spaceDatabases: [],
                    schemaInfo: null
                });
            }
            return success;
        } catch (error) {
            console.error('Disconnect failed:', error);
            return false;
        }
    },

    refreshSpaceConnectionStatus: async () => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            set({ spaceConnectionStatus: null });
            return;
        }

        try {
            const status = await api.getSpaceConnectionStatus(spaceId);
            set({ spaceConnectionStatus: status });
        } catch (error) {
            // Squelch validation error if it happens during rapid switching
            console.log('Refresh connection status skipped:', error);
        }
    },

    loadSpaceDatabases: async () => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) return;

        set({ databasesLoading: true });
        try {
            const dbs = await api.getSpaceDatabasesWithAccess(spaceId);
            set({ spaceDatabases: dbs, databasesLoading: false });
        } catch (error) {
            console.error('Failed to load databases:', error);
            set({ databasesLoading: false });
        }
    },

    testConnection: async (host, port, database, username, password, trustCertificate, encrypt) => {
        try {
            return await api.testConnection(
                host,
                port,
                database,
                username,
                password,
                trustCertificate,
                encrypt
            );
        } catch (error) {
            console.error('Connection test failed:', error);
            return false;
        }
    }
});
