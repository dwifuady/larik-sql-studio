import { StateCreator } from 'zustand';
import type { Toast } from '../../components/Toast';
import * as api from '../../api';
import type { AppState } from '../index';

export interface UISlice {
    sidebarWidth: number;
    sidebarHidden: boolean;
    sidebarHoveredWhenHidden: boolean;
    sidebarView: 'tabs' | 'explorer'; // New state
    isSaving: boolean;
    createSpaceModalOpen: boolean;
    newTabSelectorOpen: boolean;
    commandPaletteOpen: boolean;
    shortcutsDialogOpen: boolean;
    snippetsDialogOpen: boolean;
    settingsDialogOpen: boolean;
    theme: 'dark' | 'light' | 'system';
    toasts: Toast[];
    validationEnabled: boolean;
    validationShowWarnings: boolean;
    validationShowInfo: boolean;
    enableStickyNotes: boolean; // Add missing prop
    maxResultRows: number; // Add missing prop

    setSidebarWidth: (width: number) => void;
    setSidebarHidden: (hidden: boolean) => void;
    setSidebarHoveredWhenHidden: (hovered: boolean) => void;
    setSidebarView: (view: 'tabs' | 'explorer') => void; // New action
    toggleSidebarHidden: () => void;
    setCreateSpaceModalOpen: (open: boolean) => void;
    setNewTabSelectorOpen: (open: boolean) => void;
    setCommandPaletteOpen: (open: boolean) => void;
    setShortcutsDialogOpen: (open: boolean) => void;
    setSnippetsDialogOpen: (open: boolean) => void;
    setSettingsDialogOpen: (open: boolean) => void;
    setTheme: (theme: 'dark' | 'light' | 'system') => void;
    toggleTheme: () => void;
    initTheme: () => void;

    addToast: (toast: Omit<Toast, 'id'>) => void;
    removeToast: (id: string) => void;

    toggleValidation: () => void;
    setValidationEnabled: (enabled: boolean) => void;
    setValidationShowWarnings: (show: boolean) => void;
    setValidationShowInfo: (show: boolean) => void;

    loadAppSettings: () => Promise<void>;
    saveAppSettings: () => Promise<void>;
}

export const createUISlice: StateCreator<AppState, [], [], UISlice> = (set, get) => ({
    sidebarWidth: 220,
    sidebarHidden: false,
    sidebarHoveredWhenHidden: false,
    sidebarView: 'tabs', // Default to tabs
    isSaving: false,
    createSpaceModalOpen: false,
    newTabSelectorOpen: false,
    commandPaletteOpen: false,
    shortcutsDialogOpen: false,
    snippetsDialogOpen: false,
    settingsDialogOpen: false,
    theme: 'system',
    toasts: [],
    validationEnabled: true,
    validationShowWarnings: true,
    validationShowInfo: true,
    enableStickyNotes: true,
    maxResultRows: 1000,

    setSidebarWidth: (width) => set({ sidebarWidth: width }),
    setSidebarHidden: (hidden) => set({ sidebarHidden: hidden }),
    setSidebarHoveredWhenHidden: (hovered) => set({ sidebarHoveredWhenHidden: hovered }),
    setSidebarView: (view) => set({ sidebarView: view }),
    toggleSidebarHidden: () => set((state) => ({ sidebarHidden: !state.sidebarHidden })),

    setCreateSpaceModalOpen: (open) => set({ createSpaceModalOpen: open }),
    setNewTabSelectorOpen: (open) => set({ newTabSelectorOpen: open }),
    setCommandPaletteOpen: (open) => set({ commandPaletteOpen: open }),
    setShortcutsDialogOpen: (open) => set({ shortcutsDialogOpen: open }),
    setSnippetsDialogOpen: (open) => set({ snippetsDialogOpen: open }),
    setSettingsDialogOpen: (open) => set({ settingsDialogOpen: open }),

    setTheme: (theme) => {
        set({ theme });
        localStorage.setItem('larik-theme', theme);
        if (theme === 'system') {
            const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
            document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
        } else {
            document.documentElement.setAttribute('data-theme', theme);
        }
    },

    toggleTheme: () => {
        const { theme } = get();
        const nextTheme = theme === 'system' ? 'dark' : theme === 'dark' ? 'light' : 'system';
        get().setTheme(nextTheme);
    },

    initTheme: () => {
        const savedTheme = localStorage.getItem('larik-theme') as 'dark' | 'light' | 'system' | null;
        if (savedTheme) {
            get().setTheme(savedTheme);
        } else {
            get().setTheme('system');
        }
    },

    addToast: (toast) => {
        const id = Math.random().toString(36).substring(2, 9);
        const newToast = { ...toast, id };
        set((state) => ({ toasts: [...state.toasts, newToast] }));

        // Auto remove after duration
        if (toast.duration !== Infinity) {
            setTimeout(() => {
                get().removeToast(id);
            }, toast.duration || 3000);
        }
    },

    removeToast: (id) => {
        set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
    },

    toggleValidation: () => {
        set((state) => ({ validationEnabled: !state.validationEnabled }));
        get().saveAppSettings();
    },
    setValidationEnabled: (enabled) => {
        set({ validationEnabled: enabled });
        get().saveAppSettings();
    },
    setValidationShowWarnings: (show) => set({ validationShowWarnings: show }),
    setValidationShowInfo: (show) => set({ validationShowInfo: show }),

    loadAppSettings: async () => {
        try {
            const settings = await api.getAppSettings();
            set({
                validationEnabled: settings.validation_enabled,
                enableStickyNotes: settings.enable_sticky_notes,
                maxResultRows: settings.max_result_rows
            });
        } catch (error) {
            console.error('Failed to load app settings:', error);
        }
    },

    saveAppSettings: async () => {
        const {
            validationEnabled,
            enableStickyNotes,
            maxResultRows
        } = get();

        // We also need activeSpaceId and activeTabId from the store, but they are in other slices.
        // Since we have access to the full AppState via get(), we can access them.
        // However, TS might complain if we don't cast or use the combined type.
        // But `get()` here returns `AppState` because of `StateCreator<AppState...`.
        const activeSpaceId = get().activeSpaceId;
        const activeTabId = get().activeTabId;

        try {
            await api.updateAppSettings(
                validationEnabled,
                activeSpaceId,
                activeTabId,
                enableStickyNotes,
                maxResultRows
            );
        } catch (error) {
            console.error('Failed to save app settings:', error);
        }
    }
});
