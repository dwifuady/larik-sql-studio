import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createTabsSlice } from '../tabsSlice';
import { createSpacesSlice } from '../spacesSlice';
import { create } from 'zustand';
import * as api from '../../../api';

// Mock the API module
vi.mock('../../../api', () => ({
    getTabsBySpace: vi.fn(),
    createTab: vi.fn(),
    updateTab: vi.fn(),
    archiveTab: vi.fn(),
    getArchivedTabsCount: vi.fn(),
    getFoldersBySpace: vi.fn(),
    toggleTabPinned: vi.fn(),
    updateSpaceLastActiveTab: vi.fn(),
    touchTab: vi.fn(),
    updateAppSettings: vi.fn(),
    getAppsSettings: vi.fn(),
    getSpaces: vi.fn(),
    getAppSettings: vi.fn(),
}));

// Create a store for testing that includes tabs and spaces (since tabs rely on spaces)
const useTestStore = create<any>((set, get, api) => ({
    ...createSpacesSlice(set, get, api),
    ...createTabsSlice(set, get, api),
    // Mock other slices required by tabsSlice
    loadArchiveCount: vi.fn(),
    loadFolders: vi.fn(),
    touchActiveTab: vi.fn(),
    saveAppSettings: vi.fn(),
}));

describe('tabsSlice', () => {
    beforeEach(() => {
        useTestStore.setState({
            tabs: [],
            activeTabId: null,
            spaces: [],
            activeSpaceId: null,
        });
        vi.clearAllMocks();
    });

    it('should create a tab', async () => {
        const spaceId = 'space-1';
        useTestStore.setState({ activeSpaceId: spaceId });

        const newTab = {
            id: 'tab-1',
            space_id: spaceId,
            title: 'New Tab',
            tab_type: 'query',
            content: null,
            is_pinned: false,
            created_at: '',
            updated_at: '',
            sort_order: 0,
        };

        (api.createTab as any).mockResolvedValue(newTab);

        const result = await useTestStore.getState().createTab('New Tab');

        expect(api.createTab).toHaveBeenCalledWith(spaceId, 'New Tab', 'query', null, null, null);
        expect(result).toEqual(newTab);
        expect(useTestStore.getState().tabs).toContainEqual(newTab);
        expect(useTestStore.getState().activeTabId).toBe('tab-1');
    });

    it('should load tabs', async () => {
        const spaceId = 'space-1';
        const tabs = [
            { id: 'tab-1', title: 'Tab 1' },
            { id: 'tab-2', title: 'Tab 2' },
        ];
        (api.getTabsBySpace as any).mockResolvedValue(tabs);

        await useTestStore.getState().loadTabs(spaceId);

        expect(api.getTabsBySpace).toHaveBeenCalledWith(spaceId);
        expect(useTestStore.getState().tabs).toEqual(tabs);
        // Should active first tab by default
        expect(useTestStore.getState().activeTabId).toBe('tab-1');
    });

    it('should set active tab', () => {
        const spaceId = 'space-1';
        useTestStore.setState({ activeSpaceId: spaceId });

        useTestStore.setState({
            tabs: [
                { id: 'tab-1', title: 'Tab 1' } as any,
                { id: 'tab-2', title: 'Tab 2' } as any,
            ],
            activeTabId: 'tab-1',
        });

        useTestStore.getState().setActiveTab('tab-2');

        expect(useTestStore.getState().activeTabId).toBe('tab-2');
        expect(useTestStore.getState().touchActiveTab).toHaveBeenCalled();
    });
});
