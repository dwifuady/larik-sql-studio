import { StateCreator } from 'zustand';
import type { TabFolder, UpdateFolderInput, PinnedTabsGrouped, FolderWithTabs, Tab } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface FoldersSlice {
    folders: TabFolder[];
    foldersLoading: boolean;

    loadFolders: (spaceId: string) => Promise<void>;
    createFolder: (spaceId: string, name: string) => Promise<TabFolder | null>;
    createFolderFromTabs: (spaceId: string, name: string, tabIds: string[]) => Promise<TabFolder | null>;
    updateFolder: (id: string, input: UpdateFolderInput) => Promise<void>;
    deleteFolder: (id: string) => Promise<void>;
    toggleFolderExpanded: (id: string) => Promise<void>;
    addTabToFolder: (tabId: string, folderId: string) => Promise<void>;
    removeTabFromFolder: (tabId: string) => Promise<void>;
    reorderFolders: (folderIds: string[]) => Promise<void>;
    getPinnedTabsGrouped: () => PinnedTabsGrouped;
}

export const createFoldersSlice: StateCreator<AppState, [], [], FoldersSlice> = (set, get) => ({
    folders: [],
    foldersLoading: false,

    loadFolders: async (spaceId) => {
        set({ foldersLoading: true });
        try {
            const folders = await api.getFoldersBySpace(spaceId);
            set({ folders, foldersLoading: false });
        } catch (error) {
            console.error('Failed to load folders:', error);
            set({ foldersLoading: false });
        }
    },

    createFolder: async (spaceId, name) => {
        try {
            const folder = await api.createFolder(spaceId, name);
            set((state) => ({ folders: [folder, ...state.folders] }));
            return folder;
        } catch (error) {
            console.error('Failed to create folder:', error);
            return null;
        }
    },

    createFolderFromTabs: async (spaceId, name, tabIds) => {
        try {
            const folder = await api.createFolderFromTabs(spaceId, name, tabIds);
            // Optimistically add the new folder to state (don't reload to preserve expanded state of other folders)
            set((state) => ({ folders: [folder, ...state.folders] }));
            // Reload tabs to get updated folder_id values
            await get().loadTabs(spaceId);
            return folder;
        } catch (error) {
            console.error('Failed to create folder from tabs:', error);
            return null;
        }
    },

    updateFolder: async (id, input) => {
        try {
            const updated = await api.updateFolder(id, input.name, input.is_expanded, input.sort_order);
            if (updated) {
                set((state) => ({
                    folders: state.folders.map((f) => (f.id === id ? updated : f)),
                }));
            }
        } catch (error) {
            console.error('Failed to update folder:', error);
        }
    },

    deleteFolder: async (id) => {
        try {
            await api.deleteFolder(id);
            set((state) => ({
                folders: state.folders.filter((f) => f.id !== id),
            }));
            // Reload tabs to update their folder_id (they become un-foldered)
            const spaceId = get().activeSpaceId;
            if (spaceId) {
                await get().loadTabs(spaceId);
            }
        } catch (error) {
            console.error('Failed to delete folder:', error);
        }
    },

    toggleFolderExpanded: async (id) => {
        const folder = get().folders.find(f => f.id === id);
        if (folder) {
            // Optimistic update
            const newExpanded = !folder.is_expanded;
            set((state) => ({
                folders: state.folders.map((f) =>
                    f.id === id ? { ...f, is_expanded: newExpanded } : f
                ),
            }));

            // Save to backend
            try {
                await api.updateFolder(id, undefined, newExpanded);
            } catch (error) {
                console.warn('Failed to save folder expansion state:', error);
                // Revert on failure
                set((state) => ({
                    folders: state.folders.map((f) =>
                        f.id === id ? { ...f, is_expanded: !newExpanded } : f
                    ),
                }));
            }
        }
    },

    addTabToFolder: async (tabId, folderId) => {
        // Optimistic update not easy here as we need backend to validate logic
        // But we can try
        try {
            await api.addTabToFolder(tabId, folderId);
            // Reload tabs to update folder_id
            const spaceId = get().activeSpaceId;
            if (spaceId) await get().loadTabs(spaceId);
        } catch (error) {
            console.error('Failed to add tab to folder:', error);
        }
    },

    removeTabFromFolder: async (tabId) => {
        try {
            await api.removeTabFromFolder(tabId);
            // Reload tabs to update folder_id
            const spaceId = get().activeSpaceId;
            if (spaceId) await get().loadTabs(spaceId);
        } catch (error) {
            console.error('Failed to remove tab from folder:', error);
        }
    },

    reorderFolders: async (folderIds) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) return;

        // Optimistic update
        const currentFolders = get().folders;
        const reordered = folderIds
            .map(id => currentFolders.find(f => f.id === id))
            .filter((f): f is typeof currentFolders[0] => f !== undefined)
            .map((f, index) => ({ ...f, sort_order: index }));

        // Add any missing folders
        const missing = currentFolders.filter(f => !folderIds.includes(f.id));
        set({ folders: [...reordered, ...missing] });

        try {
            await api.reorderFolders(spaceId, folderIds);
        } catch (error) {
            console.error('Failed to reorder folders:', error);
            await get().loadFolders(spaceId);
        }
    },

    getPinnedTabsGrouped: () => {
        const { tabs, folders } = get();
        // Only process pinned tabs
        const pinnedTabs = tabs.filter(t => t.is_pinned);

        // Sort folders by sort_order
        const sortedFolders = [...folders].sort((a, b) => a.sort_order - b.sort_order);

        // Map folder IDs to folder objects for easy lookup
        const folderMap = new Map(sortedFolders.map(f => [f.id, f]));

        // Tabs that are not in any folder
        const ungrouped: Tab[] = [];
        // Tabs grouped by folder
        const folderTabsMap = new Map<string, Tab[]>();

        // Initialize folder arrays
        sortedFolders.forEach(f => {
            folderTabsMap.set(f.id, []);
        });

        pinnedTabs.forEach(tab => {
            if (tab.folder_id && folderMap.has(tab.folder_id)) {
                folderTabsMap.get(tab.folder_id)?.push(tab);
            } else {
                ungrouped.push(tab);
            }
        });

        // Create the result structure
        const resultFolders: FolderWithTabs[] = sortedFolders.map(folder => ({
            folder,
            tabs: folderTabsMap.get(folder.id) || []
        }));

        return {
            ungrouped,
            folders: resultFolders
        };
    }
});
