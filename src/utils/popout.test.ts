import { describe, it, expect } from 'vitest';
import { applyPopoutPayload, buildPopoutPayload, getPopoutTabId, popoutLabel, popoutTitle } from './popout';
import type { QueryResult } from '../types';

function result(rows: number, overrides: Partial<QueryResult> = {}): QueryResult {
  return {
    query_id: `q-${rows}`,
    columns: [{ name: 'id', data_type: 'int', nullable: false } as QueryResult['columns'][number]],
    rows: Array.from({ length: rows }, (_, i) => [i]),
    row_count: rows,
    execution_time_ms: 1,
    error: null,
    is_complete: true,
    is_selection: false,
    statement_index: null,
    statement_text: null,
    ...overrides,
  };
}

const emptyState = () => ({
  tabQueryResults: {} as Record<string, QueryResult[]>,
  activeResultIndex: {} as Record<string, number>,
  resultCustomNames: {} as Record<string, Record<number, string>>,
  tabResultCounters: {} as Record<string, number>,
});

describe('popout utils', () => {
  it('reads the tab id from the window URL', () => {
    expect(getPopoutTabId('?popout=abc-123')).toBe('abc-123');
    expect(getPopoutTabId('')).toBeNull();
  });

  it('builds a window label per tab', () => {
    expect(popoutLabel('abc-123')).toBe('popout-abc-123');
  });

  it('joins title parts and skips missing ones', () => {
    expect(popoutTitle('Prod', 'Query 1', 'Orders')).toBe('Prod · Query 1 · Orders');
    expect(popoutTitle(undefined, 'Query 1', null)).toBe('Query 1');
  });

  it('snapshots a tab and caps rows per result', () => {
    const state = emptyState();
    state.tabQueryResults.t1 = [result(10), result(3)];
    state.activeResultIndex.t1 = 1;
    state.resultCustomNames.t1 = { 0: 'Orders' };
    state.tabResultCounters.t1 = 4;

    const payload = buildPopoutPayload(state, 't1', 5);

    expect(payload.results[0].rows).toHaveLength(5);
    expect(payload.results[0].truncated).toBe(true);
    expect(payload.results[1].rows).toHaveLength(3);
    expect(payload.results[1].truncated).toBeUndefined();
    expect(payload.activeResultIndex).toBe(1);
    expect(payload.customNames).toEqual({ 0: 'Orders' });
    expect(payload.resultCounter).toBe(4);
    // The source state is not mutated
    expect(state.tabQueryResults.t1[0].rows).toHaveLength(10);
  });

  it('handles a tab without results', () => {
    const payload = buildPopoutPayload(emptyState(), 'none', 100);
    expect(payload).toEqual({ tabId: 'none', results: [], activeResultIndex: 0, customNames: {}, resultCounter: 0 });
  });

  it('installs a payload for its tab only', () => {
    const state = emptyState();
    state.tabQueryResults.other = [result(1)];
    const payload = buildPopoutPayload({ ...emptyState(), tabQueryResults: { t1: [result(2)] } }, 't1', 100);

    const next = applyPopoutPayload(state, payload);

    expect(next.tabQueryResults.t1).toHaveLength(1);
    expect(next.tabQueryResults.other).toBe(state.tabQueryResults.other);
    expect(next.tabResultCounters.t1).toBe(1);
  });
});
