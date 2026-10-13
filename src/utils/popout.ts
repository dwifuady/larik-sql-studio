// Pop-out tab windows: shared identifiers, event names and the result
// payload handed between the main window and a pop-out (see docs/popout-tab.md).
import type { QueryResult } from '../types';

/** URL query parameter that turns a window into a pop-out for one tab. */
export const POPOUT_QUERY_PARAM = 'popout';

/** Label of the main application window (tauri.conf.json). */
export const MAIN_WINDOW_LABEL = 'main';

/** Pop-out → main: the pop-out is listening and wants its initial results. */
export const EVENT_POPOUT_READY = 'popout:ready';
/** Main → pop-out: initial results for the tab. */
export const EVENT_POPOUT_INIT = 'popout:init';
/** Pop-out → main: the pop-out is closing, take the tab (and its results) back. */
export const EVENT_POPOUT_DOCK = 'popout:dock';

const LABEL_PREFIX = 'popout-';

/** Window label for a tab's pop-out. Tab ids are UUIDs, which are valid label characters. */
export function popoutLabel(tabId: string): string {
  return `${LABEL_PREFIX}${tabId}`;
}

/** The tab id this window was opened for, or null in the main window. */
export function getPopoutTabId(search: string = window.location.search): string | null {
  return new URLSearchParams(search).get(POPOUT_QUERY_PARAM);
}

let popoutWindow: boolean | null = null;

/** Whether this JS context belongs to a pop-out window rather than the main window. */
export function isPopoutWindow(): boolean {
  if (popoutWindow === null) {
    popoutWindow = typeof window !== 'undefined' && getPopoutTabId() !== null;
  }
  return popoutWindow;
}

/** "Space · Tab · Database", used for the pop-out's title bar and OS window title. */
export function popoutTitle(spaceName: string | null | undefined, tabTitle: string, database: string | null | undefined): string {
  return [spaceName, tabTitle, database].filter((part): part is string => !!part).join(' · ');
}

/** Results state for one tab, moved between windows when popping out and docking back. */
export interface PopoutResultsPayload {
  tabId: string;
  results: QueryResult[];
  activeResultIndex: number;
  customNames: Record<number, string>;
  resultCounter: number;
}

interface ResultsSource {
  tabQueryResults: Record<string, QueryResult[]>;
  activeResultIndex: Record<string, number>;
  resultCustomNames: Record<string, Record<number, string>>;
  tabResultCounters: Record<string, number>;
}

/**
 * Snapshot a tab's results for transfer to another window. Rows are capped at
 * `maxRows` per result so a grid that loaded extra pages doesn't flood the IPC
 * channel.
 */
export function buildPopoutPayload(state: ResultsSource, tabId: string, maxRows: number): PopoutResultsPayload {
  const results = (state.tabQueryResults[tabId] ?? []).map((r) =>
    r.rows.length > maxRows ? { ...r, rows: r.rows.slice(0, maxRows), truncated: true } : r
  );
  const activeResultIndex = Math.min(state.activeResultIndex[tabId] ?? 0, Math.max(results.length - 1, 0));
  return {
    tabId,
    results,
    activeResultIndex,
    customNames: state.resultCustomNames[tabId] ?? {},
    resultCounter: state.tabResultCounters[tabId] ?? results.length,
  };
}

/** Store patch that installs a transferred payload as the tab's results. */
export function applyPopoutPayload(state: ResultsSource, payload: PopoutResultsPayload): ResultsSource {
  const { tabId } = payload;
  return {
    tabQueryResults: { ...state.tabQueryResults, [tabId]: payload.results },
    activeResultIndex: { ...state.activeResultIndex, [tabId]: payload.activeResultIndex },
    resultCustomNames: { ...state.resultCustomNames, [tabId]: payload.customNames },
    tabResultCounters: { ...state.tabResultCounters, [tabId]: payload.resultCounter },
  };
}
