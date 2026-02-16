import { StateCreator } from 'zustand';
import type { Toast } from '../../components/Toast';
import * as api from '../../api';
import type { AppState } from '../index';

export interface UISlice {
    sidebarWidth: number;
    sidebarHidden: boolean;
    sidebarHoveredWhenHidden: boolean;
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

    setSidebarWidth: (width: number) => void;
    setSidebarHidden: (hidden: boolean) => void;
    setSidebarHoveredWhenHidden: (hovered: boolean) => void;
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

    setSidebarWidth: (width) => set({ sidebarWidth: width }),
    setSidebarHidden: (hidden) => set({ sidebarHidden: hidden }),
    setSidebarHoveredWhenHidden: (hovered) => set({ sidebarHoveredWhenHidden: hovered }),
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

    toggleValidation: () => set((state) => ({ validationEnabled: !state.validationEnabled })),
    setValidationEnabled: (enabled) => set({ validationEnabled: enabled }),
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
            activeSpaceId,
            activeTabId,
            enableStickyNotes,
            maxResultRows
        } = get();

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
