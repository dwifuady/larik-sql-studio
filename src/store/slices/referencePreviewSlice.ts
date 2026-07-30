import { StateCreator } from 'zustand';
import type { CellValue, QueryResult } from '../../types';
import type { ForeignKeyReference } from '../../utils/foreignKeyResolver';
import { formatSqlLiteral, qualifiedName, quoteIdentifier } from '../../utils/sqlLiteral';
import * as api from '../../api';
import type { AppState } from '../index';

/** One equality predicate against the referenced table. */
export interface ReferenceFilter {
    /** Column on the referenced (target) table. */
    targetColumn: string;
    /** Value taken from the source row. */
    value: CellValue;
    /** Data type of the source column, used to format the literal. */
    dataType: string;
}

/** Everything the panel needs to preview one cell's reference. */
export interface ReferenceRequest {
    tabId: string;
    reference: ForeignKeyReference;
    filters: ReferenceFilter[];
    /** FK columns we could not fill in because they are not in the result set. */
    skippedColumns: string[];
    /** The value of the clicked cell (for display). */
    value: CellValue;
}

/** 'list' shows the whole (small) reference table, 'lookup' only matching rows. */
export type ReferenceMode = 'list' | 'lookup';

export interface ReferencePreviewState {
    requestKey: string | null;
    target: { schema: string; table: string } | null;
    keyColumn: string | null;
    constraintName: string | null;
    value: CellValue;
    filters: ReferenceFilter[];
    skippedColumns: string[];
    /** True when the source cell is NULL — nothing can match. */
    hasNullValue: boolean;
    mode: ReferenceMode | null;
    /** Row count estimate from sys.partitions; null when unavailable. */
    estimatedRows: number | null;
    rowLimit: number;
    database: string | null;
    query: string | null;
    result: QueryResult | null;
    /** Index into result.rows of the row matching the source value. */
    matchedRowIndex: number | null;
    truncated: boolean;
    loading: boolean;
    error: string | null;
}

export interface ReferencePreviewSlice {
    referencePreview: ReferencePreviewState;
    referencePreviewRowLimit: number;

    setReferencePreviewRowLimit: (limit: number) => void;
    openReferencePreview: (request: ReferenceRequest) => Promise<void>;
    reloadReferencePreview: () => Promise<void>;
    /** Load the first N rows of a reference table that was too large for list mode. */
    loadReferenceListAnyway: () => Promise<void>;
    openReferenceInTab: () => Promise<void>;
    clearReferencePreview: () => void;
}

const initialReferencePreview: ReferencePreviewState = {
    requestKey: null,
    target: null,
    keyColumn: null,
    constraintName: null,
    value: null,
    filters: [],
    skippedColumns: [],
    hasNullValue: false,
    mode: null,
    estimatedRows: null,
    rowLimit: 200,
    database: null,
    query: null,
    result: null,
    matchedRowIndex: null,
    truncated: false,
    loading: false,
    error: null,
};

/** Rows returned for a lookup, capped so a partially-filtered composite key
 *  can't drag in a large slice of a big table. */
const LOOKUP_MAX_ROWS = 25;
const MAX_CACHED_RESULTS = 15;

/** Row-count estimates are metadata-only and stable enough to cache per session. */
const estimateCache = new Map<string, number | null>();

interface CachedResult {
    mode: ReferenceMode;
    query: string;
    result: QueryResult;
    matchedRowIndex: number | null;
    truncated: boolean;
    estimatedRows: number | null;
}
const resultCache = new Map<string, CachedResult>();

// Monotonic token so a slow reference query can't overwrite a newer one.
let referenceToken = 0;

function cacheResult(key: string, entry: CachedResult): void {
    resultCache.set(key, entry);
    if (resultCache.size > MAX_CACHED_RESULTS) {
        const oldest = resultCache.keys().next().value;
        if (oldest !== undefined) resultCache.delete(oldest);
    }
}

/** Cache scope: a table only means the same thing within one space + database. */
function scopeKey(spaceId: string | null, database: string | null): string {
    return `${spaceId ?? ''}@${database ?? ''}`;
}

export function buildRequestKey(
    request: ReferenceRequest,
    spaceId: string | null,
    database: string | null
): string {
    const { reference } = request;
    const filters = request.filters
        .map((filter) => `${filter.targetColumn.toLowerCase()}=${String(filter.value)}`)
        .join('&');
    return [
        scopeKey(spaceId, database),
        `${reference.targetSchema}.${reference.targetTable}`.toLowerCase(),
        reference.targetColumn.toLowerCase(),
        filters,
    ].join('|');
}

function buildWhereClause(filters: ReferenceFilter[]): { sql: string | null; unusable: string[] } {
    const predicates: string[] = [];
    const unusable: string[] = [];

    for (const filter of filters) {
        const column = quoteIdentifier(filter.targetColumn);
        if (filter.value === null || filter.value === undefined) {
            predicates.push(`${column} IS NULL`);
            continue;
        }
        const literal = formatSqlLiteral(filter.value, filter.dataType);
        if (literal === null) {
            unusable.push(filter.targetColumn);
            continue;
        }
        predicates.push(`${column} = ${literal}`);
    }

    return {
        sql: predicates.length > 0 ? predicates.join(' AND ') : null,
        unusable,
    };
}

function valuesMatch(a: CellValue, b: CellValue): boolean {
    if (a === null || a === undefined || b === null || b === undefined) {
        return (a === null || a === undefined) && (b === null || b === undefined);
    }
    if (typeof a === 'number' && typeof b === 'number') return a === b;

    const left = String(a).trim();
    const right = String(b).trim();
    if (left === right) return true;

    const leftNumber = Number(left);
    const rightNumber = Number(right);
    if (left !== '' && right !== '' && !Number.isNaN(leftNumber) && !Number.isNaN(rightNumber)) {
        return leftNumber === rightNumber;
    }

    return left.toLowerCase() === right.toLowerCase();
}

/** Find the row in a reference result that the source cell points at. */
export function findMatchedRowIndex(result: QueryResult | null, filters: ReferenceFilter[]): number | null {
    if (!result || result.rows.length === 0 || filters.length === 0) return null;

    const columnIndex = new Map<string, number>();
    result.columns.forEach((column, index) => columnIndex.set(column.name.toLowerCase(), index));

    for (let rowIdx = 0; rowIdx < result.rows.length; rowIdx++) {
        const row = result.rows[rowIdx];
        let allMatch = true;
        for (const filter of filters) {
            const colIdx = columnIndex.get(filter.targetColumn.toLowerCase());
            if (colIdx === undefined) {
                allMatch = false;
                break;
            }
            if (!valuesMatch(row[colIdx], filter.value)) {
                allMatch = false;
                break;
            }
        }
        if (allMatch) return rowIdx;
    }

    return null;
}

export const createReferencePreviewSlice: StateCreator<AppState, [], [], ReferencePreviewSlice> = (set, get) => {
    /** Run one statement and unwrap the single result set. */
    const runQuery = async (sql: string, database: string | null, maxRows: number): Promise<QueryResult> => {
        const spaceId = get().activeSpaceId;
        if (!spaceId) throw new Error('No active space');

        const results = await api.executeQuery(spaceId, sql, database, null, maxRows);
        const result = results[0];
        if (!result) throw new Error('No result returned');
        if (result.error) throw new Error(result.error);
        return result;
    };

    /** Cheap row-count estimate straight from catalog metadata (no table scan). */
    const estimateKey = (target: { schema: string; table: string }, database: string | null): string =>
        `${scopeKey(get().activeSpaceId, database)}|${target.schema.toLowerCase()}.${target.table.toLowerCase()}`;

    const estimateRowCount = async (
        target: { schema: string; table: string },
        database: string | null
    ): Promise<number | null> => {
        const key = estimateKey(target, database);
        if (estimateCache.has(key)) return estimateCache.get(key) ?? null;

        const objectName = qualifiedName(target.schema, target.table).replace(/'/g, "''");
        const sql = `SELECT ISNULL(SUM(p.[rows]), -1) AS [estimated_rows] FROM sys.partitions p WHERE p.object_id = OBJECT_ID(N'${objectName}') AND p.index_id IN (0, 1)`;

        let estimate: number | null = null;
        try {
            const result = await runQuery(sql, database, 1);
            const raw = result.rows[0]?.[0];
            const parsed = typeof raw === 'number' ? raw : Number(raw);
            estimate = Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
        } catch {
            // Metadata may be unreadable (permissions); fall back to "unknown".
            estimate = null;
        }

        estimateCache.set(key, estimate);
        return estimate;
    };

    const resolveDatabase = (tabId: string): string | null => {
        const tab = get().tabs.find((candidate) => candidate.id === tabId);
        return tab?.database ?? get().schemaInfo?.database_name ?? null;
    };

    const buildListQuery = (
        target: { schema: string; table: string },
        keyColumn: string,
        limit: number
    ): string =>
        `SELECT TOP (${limit}) * FROM ${qualifiedName(target.schema, target.table)} ORDER BY ${quoteIdentifier(keyColumn)}`;

    const buildLookupQuery = (
        target: { schema: string; table: string },
        where: string,
        limit: number
    ): string =>
        `SELECT TOP (${limit}) * FROM ${qualifiedName(target.schema, target.table)} WHERE ${where}`;

    /**
     * Fetch reference rows for the current state.
     * `forceList` is the user explicitly opting into loading a big table.
     */
    const load = async (options: { forceList?: boolean; bypassCache?: boolean } = {}): Promise<void> => {
        const state = get().referencePreview;
        if (!state.target || !state.keyColumn || !state.requestKey) return;

        const target = state.target;
        const keyColumn = state.keyColumn;
        const requestKey = state.requestKey;
        const database = state.database;
        const rowLimit = state.rowLimit;
        const cacheKey = `${requestKey}|${options.forceList ? 'list' : 'auto'}`;

        if (!options.bypassCache) {
            const cached = resultCache.get(cacheKey);
            if (cached) {
                set((current) => ({
                    referencePreview: {
                        ...current.referencePreview,
                        ...cached,
                        loading: false,
                        error: null,
                    },
                }));
                return;
            }
        }

        // A NULL foreign key value can't reference a row — don't query at all
        // unless the user explicitly asked to browse the table.
        if (state.hasNullValue && !options.forceList) {
            set((current) => ({
                referencePreview: { ...current.referencePreview, loading: false, mode: 'lookup', error: null },
            }));
            return;
        }

        const token = ++referenceToken;
        set((current) => ({
            referencePreview: { ...current.referencePreview, loading: true, error: null },
        }));

        try {
            let estimate = state.estimatedRows;
            if (!options.forceList) {
                estimate = await estimateRowCount(target, database);
                if (token !== referenceToken) return;
            }

            const { sql: where, unusable } = buildWhereClause(state.filters);
            // List mode only when we know the table is small, or the user asked for it.
            const useList = options.forceList || (estimate !== null && estimate <= rowLimit);
            const mode: ReferenceMode = useList ? 'list' : 'lookup';

            let query: string;
            let maxRows: number;
            if (useList) {
                query = buildListQuery(target, keyColumn, rowLimit);
                maxRows = rowLimit;
            } else if (where) {
                maxRows = Math.min(rowLimit, LOOKUP_MAX_ROWS);
                query = buildLookupQuery(target, where, maxRows);
            } else {
                // Nothing usable to filter on and the table is too big to list.
                set((current) => ({
                    referencePreview: {
                        ...current.referencePreview,
                        loading: false,
                        mode: 'lookup',
                        estimatedRows: estimate,
                        query: null,
                        result: null,
                        matchedRowIndex: null,
                        truncated: false,
                        skippedColumns: Array.from(
                            new Set([...current.referencePreview.skippedColumns, ...unusable])
                        ),
                    },
                }));
                return;
            }

            const result = await runQuery(query, database, maxRows);
            if (token !== referenceToken) return;

            const matchedRowIndex = findMatchedRowIndex(result, state.filters);
            const entry: CachedResult = {
                mode,
                query,
                result,
                matchedRowIndex,
                truncated: Boolean(result.truncated) || result.rows.length >= maxRows,
                estimatedRows: estimate,
            };
            cacheResult(cacheKey, entry);

            set((current) => ({
                referencePreview: {
                    ...current.referencePreview,
                    ...entry,
                    skippedColumns: unusable.length > 0
                        ? Array.from(new Set([...current.referencePreview.skippedColumns, ...unusable]))
                        : current.referencePreview.skippedColumns,
                    loading: false,
                    error: null,
                },
            }));
        } catch (error) {
            if (token !== referenceToken) return;
            set((current) => ({
                referencePreview: {
                    ...current.referencePreview,
                    loading: false,
                    error: error instanceof Error ? error.message : String(error),
                },
            }));
        }
    };

    return {
        referencePreview: initialReferencePreview,
        referencePreviewRowLimit: 200,

        setReferencePreviewRowLimit: (limit) => {
            const clamped = Math.max(10, Math.min(5000, Math.round(limit) || 200));
            set({ referencePreviewRowLimit: clamped });
            resultCache.clear();
            get().saveAppSettings();
        },

        openReferencePreview: async (request) => {
            const database = resolveDatabase(request.tabId);
            const requestKey = buildRequestKey(request, get().activeSpaceId, database);
            const existing = get().referencePreview;

            // Same cell as before and we already have data — nothing to do.
            if (existing.requestKey === requestKey && (existing.result || existing.error || existing.loading)) {
                return;
            }

            const { reference } = request;
            set({
                referencePreview: {
                    ...initialReferencePreview,
                    requestKey,
                    target: { schema: reference.targetSchema, table: reference.targetTable },
                    keyColumn: reference.targetColumn,
                    constraintName: reference.constraintName,
                    value: request.value,
                    filters: request.filters,
                    skippedColumns: request.skippedColumns,
                    hasNullValue: request.value === null || request.value === undefined,
                    rowLimit: get().referencePreviewRowLimit,
                    database,
                },
            });

            await load();
        },

        reloadReferencePreview: async () => {
            const state = get().referencePreview;
            if (!state.target) return;
            estimateCache.delete(estimateKey(state.target, state.database));
            await load({ bypassCache: true, forceList: state.mode === 'list' });
        },

        loadReferenceListAnyway: async () => {
            await load({ forceList: true, bypassCache: false });
        },

        openReferenceInTab: async () => {
            const state = get().referencePreview;
            if (!state.target) return;

            const query =
                state.query ??
                buildListQuery(state.target, state.keyColumn ?? '', state.rowLimit);

            const tab = await get().createTab(state.target.table, 'query', query, state.database);
            if (tab) {
                await get().executeQuery(tab.id, query);
            }
        },

        clearReferencePreview: () => {
            referenceToken++;
            set({ referencePreview: initialReferencePreview });
        },
    };
};
