import { StateCreator } from 'zustand';
import { WebviewWindow, getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { emitTo } from '@tauri-apps/api/event';
import * as api from '../../api';
import type { AppState } from '../index';
import {
    EVENT_POPOUT_DOCK,
    EVENT_POPOUT_INIT,
    EVENT_POPOUT_READY,
    applyPopoutPayload,
    buildPopoutPayload,
    popoutLabel,
    popoutTitle,
    type PopoutResultsPayload,
} from '../../utils/popout';
import { flushPendingAutosave } from '../../utils/pendingAutosave';

const POPOUT_WIDTH = 760;
const POPOUT_HEIGHT = 560;

/** Main-window side of pop-out tabs (see docs/popout-tab.md). */
export interface PopoutSlice {
    /** Tabs currently owned by a pop-out window: tab id → space id. */
    poppedOutTabs: Record<string, string>;

    /** Open the tab in its own always-on-top window, or focus it if already popped out. */
    popOutTab: (tabId: string) => Promise<void>;
    focusPopout: (tabId: string) => Promise<void>;
    /**
     * Close a tab's pop-out from the main window. By default the pop-out sends
     * its results back as it closes; with `discard` (tab archived, space
     * deleted) they are dropped.
     */
    closePopout: (tabId: string, options?: { discard?: boolean; reason?: string }) => Promise<void>;
    closePopoutsForSpace: (spaceId: string, reason?: string) => Promise<void>;
    /** Listen for pop-out handshake events. Main window only; returns the cleanup. */
    initPopoutHost: () => Promise<() => void>;
}

export const createPopoutSlice: StateCreator<AppState, [], [], PopoutSlice> = (set, get) => {
    // Tabs whose dock event is being applied; their window may already be gone
    const docking = new Set<string>();

    const unmark = (tabId: string) => {
        if (!(tabId in get().poppedOutTabs)) return;
        set((state) => {
            const { [tabId]: _removed, ...rest } = state.poppedOutTabs;
            return { poppedOutTabs: rest };
        });
    };

    return {
        poppedOutTabs: {},

        popOutTab: async (tabId) => {
            if (tabId in get().poppedOutTabs) {
                await get().focusPopout(tabId);
                return;
            }

            const tab = get().tabs.find(t => t.id === tabId);
            if (!tab) return;

            if (get().tabExecuting[tabId]) {
                get().addToast({ type: 'info', message: 'Wait for the query to finish before popping out this tab' });
                return;
            }

            // The pop-out loads the tab from storage, so write pending edits first
            await flushPendingAutosave(tabId);

            const space = get().spaces.find(s => s.id === tab.space_id);
            const label = popoutLabel(tabId);
            set((state) => ({ poppedOutTabs: { ...state.poppedOutTabs, [tabId]: tab.space_id } }));

            const win = new WebviewWindow(label, {
                url: `index.html?popout=${encodeURIComponent(tabId)}`,
                title: popoutTitle(space?.name, tab.title, tab.database ?? space?.connection_database),
                width: POPOUT_WIDTH,
                height: POPOUT_HEIGHT,
                minWidth: 420,
                minHeight: 280,
                decorations: false,
                alwaysOnTop: true,
                focus: true,
            });

            void win.once('tauri://error', (e) => {
                console.error('[Popout] Failed to create window:', e.payload);
                unmark(tabId);
                get().addToast({ type: 'error', message: 'Could not open the pop-out window' });
            });
            // Fallback for a pop-out that goes away without docking (e.g. a crash)
            void win.once('tauri://destroyed', () => {
                if (!docking.has(tabId)) unmark(tabId);
            });
        },

        focusPopout: async (tabId) => {
            const win = await WebviewWindow.getByLabel(popoutLabel(tabId));
            if (!win) {
                unmark(tabId);
                return;
            }
            await win.unminimize();
            await win.setFocus();
        },

        closePopout: async (tabId, options = {}) => {
            if (!(tabId in get().poppedOutTabs)) return;
            const tab = get().tabs.find(t => t.id === tabId);
            if (options.discard) {
                // Unmark first so the dock event the pop-out sends while closing is ignored
                unmark(tabId);
            }
            const win = await WebviewWindow.getByLabel(popoutLabel(tabId));
            if (win) {
                await win.close();
            } else {
                unmark(tabId);
            }
            if (options.reason) {
                get().addToast({
                    type: 'info',
                    message: tab ? `Closed the pop-out for "${tab.title}": ${options.reason}` : `Closed a pop-out: ${options.reason}`,
                });
            }
        },

        closePopoutsForSpace: async (spaceId, reason) => {
            const tabIds = Object.entries(get().poppedOutTabs)
                .filter(([, owner]) => owner === spaceId)
                .map(([tabId]) => tabId);
            await Promise.all(tabIds.map(tabId => get().closePopout(tabId, { discard: true, reason })));
        },

        initPopoutHost: async () => {
            const host = getCurrentWebviewWindow();

            const unlistenReady = await host.listen<{ tabId: string }>(EVENT_POPOUT_READY, (event) => {
                const { tabId } = event.payload;
                if (!(tabId in get().poppedOutTabs)) return;
                const payload = buildPopoutPayload(get(), tabId, get().maxResultRows);
                void emitTo(popoutLabel(tabId), EVENT_POPOUT_INIT, payload);
            });

            const unlistenDock = await host.listen<PopoutResultsPayload>(EVENT_POPOUT_DOCK, async (event) => {
                const payload = event.payload;
                const { tabId } = payload;
                if (!(tabId in get().poppedOutTabs) || docking.has(tabId)) return;
                docking.add(tabId);

                // The pop-out autosaved to storage. Reload the tab before unmarking it,
                // because the main editor mounts with whatever content the store holds.
                let fresh: Awaited<ReturnType<typeof api.getTab>> = null;
                try {
                    fresh = await api.getTab(tabId);
                } catch (error) {
                    console.error('[Popout] Failed to reload docked tab:', error);
                }

                set((state) => {
                    const { [tabId]: _removed, ...poppedOutTabs } = state.poppedOutTabs;
                    return {
                        ...applyPopoutPayload(state, payload),
                        tabExecuting: { ...state.tabExecuting, [tabId]: false },
                        tabs: fresh ? state.tabs.map(t => (t.id === tabId ? fresh! : t)) : state.tabs,
                        poppedOutTabs,
                    };
                });
                docking.delete(tabId);
            });

            return () => {
                unlistenReady();
                unlistenDock();
            };
        },
    };
};
