import { describe, it, expect, vi, beforeEach } from 'vitest';
import { create } from 'zustand';
import { createPopoutSlice } from '../popoutSlice';
import { createQueriesSlice } from '../queriesSlice';
import * as api from '../../../api';
import { emitTo } from '@tauri-apps/api/event';
import { setPendingAutosave } from '../../../utils/pendingAutosave';

const mocks = vi.hoisted(() => {
    type Handler = (event: { payload: unknown }) => unknown;
    const created: Array<{ label: string; options: Record<string, unknown>; once: ReturnType<typeof vi.fn> }> = [];
    const hostListeners: Record<string, Handler> = {};
    const existing = {
        close: vi.fn(async () => undefined),
        unminimize: vi.fn(async () => undefined),
        setFocus: vi.fn(async () => undefined),
    };
    return { created, hostListeners, existing };
});

vi.mock('@tauri-apps/api/webviewWindow', () => {
    class WebviewWindow {
        static getByLabel = vi.fn(async () => mocks.existing);
        once = vi.fn(async () => () => undefined);
        constructor(public label: string, public options: Record<string, unknown>) {
            mocks.created.push(this);
        }
    }
    return {
        WebviewWindow,
        getCurrentWebviewWindow: () => ({
            listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => unknown) => {
                mocks.hostListeners[event] = handler;
                return () => undefined;
            }),
        }),
    };
});

vi.mock('@tauri-apps/api/event', () => ({ emitTo: vi.fn(async () => undefined) }));

vi.mock('../../../api', () => ({ getTab: vi.fn() }));

const tab = {
    id: 'tab-1',
    space_id: 'space-1',
    title: 'Orders',
    tab_type: 'query',
    content: 'select 1',
    metadata: null,
    database: 'Shop',
    folder_id: null,
    is_pinned: false,
    created_at: '',
    updated_at: '',
    sort_order: 0,
};

const addToast = vi.fn();

const useTestStore = create<any>((set, get, store) => ({
    ...createQueriesSlice(set, get, store),
    ...createPopoutSlice(set, get, store),
    tabs: [tab],
    spaces: [{ id: 'space-1', name: 'Prod', color: '#ff0000', connection_database: 'Shop' }],
    addToast,
}));

const result = (rows: number) => ({
    query_id: `q-${rows}`,
    columns: [],
    rows: Array.from({ length: rows }, (_, i) => [i]),
    row_count: rows,
    execution_time_ms: 1,
    error: null,
    is_complete: true,
    is_selection: false,
    statement_index: null,
    statement_text: null,
});

describe('popoutSlice', () => {
    beforeEach(() => {
        mocks.created.length = 0;
        vi.clearAllMocks();
        useTestStore.setState({
            tabs: [tab],
            poppedOutTabs: {},
            tabQueryResults: {},
            tabExecuting: {},
            activeResultIndex: {},
            resultCustomNames: {},
            tabResultCounters: {},
            maxResultRows: 5000,
        });
    });

    it('opens an always-on-top window for the tab and marks it popped out', async () => {
        await useTestStore.getState().popOutTab('tab-1');

        expect(mocks.created).toHaveLength(1);
        const win = mocks.created[0];
        expect(win.label).toBe('popout-tab-1');
        expect(win.options).toMatchObject({
            url: 'index.html?popout=tab-1',
            title: 'Prod · Orders · Shop',
            alwaysOnTop: true,
            decorations: false,
        });
        expect(useTestStore.getState().poppedOutTabs).toEqual({ 'tab-1': 'space-1' });
    });

    it('writes a pending autosave before opening the window', async () => {
        const save = vi.fn(async () => {
            expect(mocks.created).toHaveLength(0);
        });
        setPendingAutosave('tab-1', save);

        await useTestStore.getState().popOutTab('tab-1');

        expect(save).toHaveBeenCalledOnce();
        expect(mocks.created).toHaveLength(1);
    });

    it('does not pop out a tab whose query is running', async () => {
        useTestStore.setState({ tabExecuting: { 'tab-1': true } });

        await useTestStore.getState().popOutTab('tab-1');

        expect(mocks.created).toHaveLength(0);
        expect(useTestStore.getState().poppedOutTabs).toEqual({});
        expect(addToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'info' }));
    });

    it('focuses the existing window instead of opening a second one', async () => {
        useTestStore.setState({ poppedOutTabs: { 'tab-1': 'space-1' } });

        await useTestStore.getState().popOutTab('tab-1');

        expect(mocks.created).toHaveLength(0);
        expect(mocks.existing.setFocus).toHaveBeenCalled();
    });

    it('sends the tab results when the pop-out is ready', async () => {
        await useTestStore.getState().initPopoutHost();
        useTestStore.setState({
            poppedOutTabs: { 'tab-1': 'space-1' },
            tabQueryResults: { 'tab-1': [result(10)] },
            maxResultRows: 4,
        });

        mocks.hostListeners['popout:ready']({ payload: { tabId: 'tab-1' } });

        expect(emitTo).toHaveBeenCalledWith('popout-tab-1', 'popout:init', expect.objectContaining({ tabId: 'tab-1' }));
        const payload = vi.mocked(emitTo).mock.calls[0][2] as { results: { rows: unknown[] }[] };
        expect(payload.results[0].rows).toHaveLength(4);
    });

    it('takes the tab back with its results and fresh content on dock', async () => {
        await useTestStore.getState().initPopoutHost();
        useTestStore.setState({ poppedOutTabs: { 'tab-1': 'space-1' }, tabExecuting: { 'tab-1': true } });
        vi.mocked(api.getTab).mockResolvedValue({ ...tab, content: 'select 2' } as any);

        await mocks.hostListeners['popout:dock']({
            payload: { tabId: 'tab-1', results: [result(2)], activeResultIndex: 0, customNames: { 0: 'Mine' }, resultCounter: 3 },
        });

        const state = useTestStore.getState();
        expect(state.poppedOutTabs).toEqual({});
        expect(state.tabQueryResults['tab-1']).toHaveLength(1);
        expect(state.resultCustomNames['tab-1']).toEqual({ 0: 'Mine' });
        expect(state.tabResultCounters['tab-1']).toBe(3);
        expect(state.tabExecuting['tab-1']).toBe(false);
        expect(state.tabs[0].content).toBe('select 2');
    });

    it('ignores a dock from a tab that is not popped out', async () => {
        await useTestStore.getState().initPopoutHost();

        await mocks.hostListeners['popout:dock']({
            payload: { tabId: 'tab-1', results: [result(2)], activeResultIndex: 0, customNames: {}, resultCounter: 1 },
        });

        expect(useTestStore.getState().tabQueryResults['tab-1']).toBeUndefined();
        expect(api.getTab).not.toHaveBeenCalled();
    });

    it('discards the pop-out results when closing for an archived tab', async () => {
        await useTestStore.getState().initPopoutHost();
        useTestStore.setState({ poppedOutTabs: { 'tab-1': 'space-1' } });

        await useTestStore.getState().closePopout('tab-1', { discard: true, reason: 'the tab was archived' });
        // The pop-out still sends its dock event while closing
        await mocks.hostListeners['popout:dock']({
            payload: { tabId: 'tab-1', results: [result(2)], activeResultIndex: 0, customNames: {}, resultCounter: 1 },
        });

        expect(mocks.existing.close).toHaveBeenCalled();
        expect(useTestStore.getState().poppedOutTabs).toEqual({});
        expect(useTestStore.getState().tabQueryResults['tab-1']).toBeUndefined();
        expect(addToast).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining('archived') }));
    });

    it('closes only the pop-outs of a deleted space', async () => {
        useTestStore.setState({ poppedOutTabs: { 'tab-1': 'space-1', 'tab-2': 'space-2' } });

        await useTestStore.getState().closePopoutsForSpace('space-1');

        expect(useTestStore.getState().poppedOutTabs).toEqual({ 'tab-2': 'space-2' });
    });
});
