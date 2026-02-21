import { StateCreator } from 'zustand';
import type { Tab } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface TabsSlice {
    tabs: Tab[];
    activeTabId: string | null;
    tabsLoading: boolean;

    loadTabs: (spaceId: string) => Promise<void>;
    createTab: (title: string, tabType?: string, content?: string | null, database?: string | null) => Promise<Tab | null>;
    updateTab: (id: string, updates: { title?: string; content?: string; metadata?: string }) => Promise<void>;
    deleteTab: (id: string) => Promise<void>;
    setActiveTab: (id: string | null) => void;
    nextTab: () => void;
    previousTab: () => void;
    reorderTabs: (tabIds: string[]) => Promise<void>;
    autosaveContent: (id: string, content: string) => Promise<void>;
    toggleTabPinned: (id: string) => Promise<void>;
    updateTabDatabase: (id: string, database: string | null) => Promise<void>;
    updateSpaceLastActiveTab: (spaceId: string, tabId: string | null) => Promise<void>;

    getActiveTab: () => Tab | null;
}

export const createTabsSlice: StateCreator<AppState, [], [], TabsSlice> = (set, get) => ({
    tabs: [],
    activeTabId: null,
    tabsLoading: false,

    getActiveTab: () => {
        const { tabs, activeTabId } = get();
        return tabs.find(t => t.id === activeTabId) ?? null;
    },

    loadTabs: async (spaceId) => {
        set({ tabsLoading: true });
        try {
            const tabs = await api.getTabsBySpace(spaceId);
            const spaces = get().spaces;
            const currentSpace = spaces.find(s => s.id === spaceId);

            set({ tabs, tabsLoading: false });

            // Auto-select tab: prioritize last active tab, fallback to first tab
            if (tabs.length > 0 && !get().activeTabId) {
                let targetTabId = tabs[0].id; // fallback to first tab

                // If space has a remembered last active tab, try to use it
                if (currentSpace?.last_active_tab_id) {
                    const lastActiveTab = tabs.find(t => t.id === currentSpace.last_active_tab_id);
                    if (lastActiveTab) {
                        targetTabId = lastActiveTab.id;
                    }
                }

                set({ activeTabId: targetTabId });
            }
        } catch (error) {
            console.error('Failed to load tabs:', error);
            set({ tabsLoading: false });
        }
    },

    createTab: async (title, tabType = 'query', content = null, database = null) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) {
            console.error('Cannot create tab: no active space');
            return null;
        }

        try {
            // Auto-select database for SQLite spaces
            const activeSpace = get().spaces.find(s => s.id === spaceId);
            let targetDatabase = database;
            if (!targetDatabase && activeSpace?.database_type?.toLowerCase() === 'sqlite') {
                const spaceDatabases = get().spaceDatabases;
                if (spaceDatabases && spaceDatabases.length > 0) {
                    targetDatabase = spaceDatabases[0].name;
                }
            }

            const tab = await api.createTab(spaceId, title, tabType, content, null, targetDatabase);
            set((state) => ({ tabs: [tab, ...state.tabs], activeTabId: tab.id }));
            return tab;
        } catch (error) {
            console.error('Failed to create tab:', error);
            return null;
        }
    },

    updateTab: async (id, updates) => {
        const updated = await api.updateTab(id, updates.title, updates.content, updates.metadata);
        if (updated) {
            set((state) => ({
                tabs: state.tabs.map((t) => (t.id === id ? updated : t)),
            }));
        }
    },

    deleteTab: async (id) => {
        // Archive the tab instead of permanently deleting it (Arc Browser-style)
        await api.archiveTab(id);
        // Reload archive count so UI updates immediately
        await get().loadArchiveCount(get().activeSpaceId);
        set((state) => {
            const newTabs = state.tabs.filter((t) => t.id !== id);
            const newActiveTabId = state.activeTabId === id
                ? (newTabs[0]?.id ?? null)
                : state.activeTabId;

            // Clean up all state associated with this tab to prevent memory leaks
            const { [id]: _removedResults, ...newTabQueryResults } = state.tabQueryResults;
            const { [id]: _removedExecuting, ...newTabExecuting } = state.tabExecuting;
            const { [id]: _removedActiveIdx, ...newActiveResultIndex } = state.activeResultIndex;
            const { [id]: _removedNames, ...newResultCustomNames } = state.resultCustomNames;
            const { [id]: _removedOrder, ...newResultColumnOrder } = state.resultColumnOrder;
            const { [id]: _removedHidden, ...newResultsHidden } = state.resultsHidden;

            return {
                tabs: newTabs,
                activeTabId: newActiveTabId,
                tabQueryResults: newTabQueryResults,
                tabExecuting: newTabExecuting,
                activeResultIndex: newActiveResultIndex,
                resultCustomNames: newResultCustomNames,
                resultColumnOrder: newResultColumnOrder,
                resultsHidden: newResultsHidden
            };
        });
        // Reload folders to handle auto-cleanup of empty folders
        const spaceId = get().activeSpaceId;
        if (spaceId) {
            await get().loadFolders(spaceId);
        }
    },

    setActiveTab: (id) => {
        const prevTabId = get().activeTabId;
        set({ activeTabId: id });

        // Update activity tracking
        get().touchActiveTab();

        // Save last active tab for the current space (only if tab actually changed)
        if (id !== prevTabId) {
            const activeSpaceId = get().activeSpaceId;
            if (activeSpaceId) {
                // Update backend and local state using the store method
                get().updateSpaceLastActiveTab(activeSpaceId, id).catch(error => {
                    console.error('Failed to update last active tab:', error);
                });
            }
        }

        // Persist last opened tab
        get().saveAppSettings();
    },

    nextTab: () => {
        const { tabs, activeTabId } = get();
        if (tabs.length === 0) return;

        const currentIndex = tabs.findIndex(t => t.id === activeTabId);
        const nextIndex = (currentIndex + 1) % tabs.length;
        get().setActiveTab(tabs[nextIndex].id);
    },

    previousTab: () => {
        const { tabs, activeTabId } = get();
        if (tabs.length === 0) return;

        const currentIndex = tabs.findIndex(t => t.id === activeTabId);
        const previousIndex = currentIndex <= 0 ? tabs.length - 1 : currentIndex - 1;
        get().setActiveTab(tabs[previousIndex].id);
    },

    reorderTabs: async (tabIds) => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) return;

        // Optimistic update: reorder local state immediately
        const currentTabs = get().tabs;
        const reorderedTabs = tabIds
            .map(id => currentTabs.find(t => t.id === id))
            .filter((t): t is typeof currentTabs[0] => t !== undefined)
            .map((tab, index) => ({ ...tab, sort_order: index }));

        // Merge with tabs not in the reorder list (maintain their order)
        const tabsNotInList = currentTabs.filter(t => !tabIds.includes(t.id));
        set({ tabs: [...reorderedTabs, ...tabsNotInList] });

        // Save to backend in background (don't reload)
        api.reorderTabs(spaceId, tabIds).catch(err => {
            console.error('Failed to save tab order:', err);
            // On error, reload to get correct order
            get().loadTabs(spaceId);
        });
    },

    autosaveContent: async (id, content) => {
        set({ isSaving: true });
        try {
            await api.autosaveTabContent(id, content);
            // Update local state without full reload
            set((state) => ({
                tabs: state.tabs.map((t) =>
                    t.id === id ? { ...t, content } : t
                ),
                isSaving: false,
            }));
        } catch (error) {
            console.error('Autosave failed:', error);
            set({ isSaving: false });
        }
    },

    toggleTabPinned: async (id) => {
        try {
            const updated = await api.toggleTabPinned(id);
            if (updated) {
                // Update and re-sort tabs (pinned first)
                set((state) => {
                    const newTabs = state.tabs.map((t) => (t.id === id ? updated : t));
                    // Sort: pinned tabs first, then by sort_order
                    newTabs.sort((a, b) => {
                        if (a.is_pinned !== b.is_pinned) {
                            return a.is_pinned ? -1 : 1;
                        }
                        return a.sort_order - b.sort_order;
                    });
                    return { tabs: newTabs };
                });
            }
        } catch (error) {
            console.error('Toggle pin failed:', error);
        }
    },

    updateTabDatabase: async (id, database) => {
        try {
            const success = await api.updateTabDatabase(id, database);
            if (success) {
                // Update local state with the new database value
                set((state) => ({
                    tabs: state.tabs.map((t) => (t.id === id ? { ...t, database } : t)),
                }));
            }
        } catch (error) {
            console.error('Failed to update tab database:', error);
        }
    },

    updateSpaceLastActiveTab: async (spaceId, tabId) => {
        try {
            await api.updateSpaceLastActiveTab(spaceId, tabId);
            // Update the local space state to reflect the change
            set((state) => ({
                spaces: state.spaces.map((s) =>
                    s.id === spaceId ? { ...s, last_active_tab_id: tabId } : s
                ),
            }));
        } catch (error) {
            console.error('Failed to update space last active tab:', error);
        }
    },
});
