import { StateCreator } from 'zustand';
import type { ArchivedTab, ArchiveSearchResult, AutoArchiveSettings } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface ArchiveSlice {
    archivedTabs: ArchivedTab[];
    archivedTabsLoading: boolean;
    archiveSearchResults: ArchiveSearchResult[] | null;
    archiveSearching: boolean;
    archivedTabsCount: number;
    archiveModalOpen: boolean;
    autoArchiveSettings: AutoArchiveSettings;

    loadArchivedTabs: (spaceId?: string | null) => Promise<void>;
    searchArchive: (query: string) => Promise<void>;
    archiveTab: (tabId: string) => Promise<void>;
    restoreTab: (archivedId: string, targetSpaceId?: string) => Promise<void>;
    deleteArchivedTab: (archivedId: string) => Promise<void>;
    loadArchiveCount: (spaceId?: string | null) => Promise<void>;
    setArchiveModalOpen: (open: boolean) => void;
    loadAutoArchiveSettings: () => Promise<void>;
    updateAutoArchiveSettings: (enabled: boolean, days: number) => Promise<void>;
    touchActiveTab: () => Promise<void>;
}

export const createArchiveSlice: StateCreator<AppState, [], [], ArchiveSlice> = (set, get) => ({
    archivedTabs: [],
    archivedTabsLoading: false,
    archiveSearchResults: null,
    archiveSearching: false,
    archivedTabsCount: 0,
    archiveModalOpen: false,
    autoArchiveSettings: { enabled: true, days_inactive: 14 },

    loadArchivedTabs: async (spaceId) => {
        set({ archivedTabsLoading: true });
        try {
            // If spaceId not provided, use active space (or null for all?)
            // Backend likely supports filtering by space_id
            const idToUse = spaceId === undefined ? get().activeSpaceId : spaceId;
            const tabs = await api.getArchivedTabs(idToUse);
            set({ archivedTabs: tabs, archivedTabsLoading: false });
        } catch (error) {
            console.error('Failed to load archived tabs:', error);
            set({ archivedTabsLoading: false });
        }
    },

    searchArchive: async (query) => {
        if (!query.trim()) {
            set({ archiveSearchResults: null });
            return;
        }

        set({ archiveSearching: true });
        try {
            const results = await api.searchArchivedTabs(query);
            set({ archiveSearchResults: results, archiveSearching: false });
        } catch (error) {
            console.error('Failed to search archive:', error);
            set({ archiveSearching: false });
        }
    },

    archiveTab: async (tabId) => {
        // This is essentially same as deleteTab but explicit
        await get().deleteTab(tabId);
    },

    restoreTab: async (archivedId, targetSpaceId) => {
        try {
            // If no target space, use active space
            const spaceId = targetSpaceId || get().activeSpaceId;
            if (!spaceId) {
                throw new Error('No active space to restore tab to');
            }

            await api.restoreArchivedTab(archivedId, spaceId);

            // Reload lists
            await get().loadTabs(spaceId);
            await get().loadArchivedTabs(spaceId);
            await get().loadArchiveCount(spaceId);

            // If we are searching, refresh search
            if (get().archiveSearchResults) {
                set((state) => ({
                    archiveSearchResults: state.archiveSearchResults?.filter(r => r.archived_tab.id !== archivedId) ?? null
                }));
            }
        } catch (error) {
            console.error('Failed to restore tab:', error);
        }
    },

    deleteArchivedTab: async (archivedId) => {
        try {
            await api.deleteArchivedTab(archivedId);
            // Update local state
            set((state) => ({
                archivedTabs: state.archivedTabs.filter(t => t.id !== archivedId),
                archivedTabsCount: Math.max(0, state.archivedTabsCount - 1),
                archiveSearchResults: state.archiveSearchResults
                    ? state.archiveSearchResults.filter(r => r.archived_tab.id !== archivedId)
                    : null
            }));
        } catch (error) {
            console.error('Failed to delete archived tab:', error);
        }
    },

    loadArchiveCount: async (spaceId) => {
        try {
            const idToUse = spaceId === undefined ? get().activeSpaceId : spaceId;
            const count = await api.getArchivedTabsCount(idToUse);
            set({ archivedTabsCount: count });
        } catch (error) {
            console.error('Failed to load archive count:', error);
        }
    },

    setArchiveModalOpen: (open) => {
        set({ archiveModalOpen: open });
        if (open) {
            // Load tabs when opening
            get().loadArchivedTabs(get().activeSpaceId);
        } else {
            // Clear search when closing
            set({ archiveSearchResults: null });
        }
    },

    loadAutoArchiveSettings: async () => {
        try {
            const settings = await api.getAutoArchiveSettings();
            set({ autoArchiveSettings: settings });
        } catch (error) {
            console.error('Failed to load auto-archive settings:', error);
        }
    },

    updateAutoArchiveSettings: async (enabled, days) => {
        try {
            await api.updateAutoArchiveSettings(enabled, days);
            set({ autoArchiveSettings: { enabled, days_inactive: days } });
        } catch (error) {
            console.error('Failed to update auto-archive settings:', error);
        }
    },

    touchActiveTab: async () => {
        const tabId = get().activeTabId;
        if (tabId) {
            try {
                await api.touchTab(tabId);
            } catch (error) {
                // Ignore errors for touch/activity tracking
            }
        }
    }
});
