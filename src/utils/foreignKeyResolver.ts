import type { SchemaInfo, SchemaRelationshipInfo, TableInfo, VirtualReference } from '../types';
import { extractReferencedTables } from './sqlAstExtractor';

/** One column of a foreign key constraint. */
export interface ReferenceColumnPair {
  sourceColumn: string;
  targetColumn: string;
}

/** A resolved reference for a single result-set column. */
export interface ForeignKeyReference {
  /** Constraint name for a real FK; null for a user-defined reference. */
  constraintName: string | null;
  sourceSchema: string;
  sourceTable: string;
  targetSchema: string;
  targetTable: string;
  /** All column pairs of the constraint, in ordinal order. */
  columnPairs: ReferenceColumnPair[];
  /** The referenced column that the clicked grid column maps to. */
  targetColumn: string;
  /** True when this came from a user-defined reference, not the database. */
  isVirtual: boolean;
  /** Id of the backing virtual reference, so the UI can edit or remove it. */
  virtualReferenceId?: string;
}

function eq(a: string | undefined, b: string | undefined): boolean {
  return (a ?? '').toLowerCase() === (b ?? '').toLowerCase();
}

function findTable(schemaInfo: SchemaInfo, schemaName: string, tableName: string): TableInfo | undefined {
  return schemaInfo.tables.find(
    (table) => eq(table.schema_name, schemaName) && eq(table.table_name, tableName)
  );
}

function tableHasColumn(table: TableInfo, columnName: string): boolean {
  return table.columns.some((column) => eq(column.name, columnName));
}

/**
 * The tables a statement reads that we hold metadata for, de-duplicated.
 * Parsing SQL is the expensive part, so callers resolving many columns should
 * do this once and reuse the result.
 */
function resolveKnownTables(sql: string, schemaInfo: SchemaInfo): TableInfo[] {
  const referenced = extractReferencedTables(sql);
  if (referenced.length === 0) return [];

  const seen = new Set<string>();
  const known: TableInfo[] = [];
  for (const ref of referenced) {
    const key = `${ref.schema.toLowerCase()}.${ref.table.toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const table = findTable(schemaInfo, ref.schema, ref.table);
    if (table) known.push(table);
  }
  return known;
}

/**
 * Pick the one table that owns `columnName`.
 *
 * A single known table means every column belongs to it. With joins the column
 * name has to be unique across the referenced tables; anything ambiguous
 * returns null so we never preview the wrong relationship.
 */
function pickSourceTable(tables: TableInfo[], columnName: string): { schema: string; table: string } | null {
  if (tables.length === 0) return null;
  const candidates = tables.filter((table) => tableHasColumn(table, columnName));
  if (candidates.length !== 1) return null;
  return { schema: candidates[0].schema_name, table: candidates[0].table_name };
}

/** Work out which table a result-set column came from. */
export function resolveColumnSourceTable(
  sql: string | null | undefined,
  columnName: string,
  schemaInfo: SchemaInfo | null
): { schema: string; table: string } | null {
  if (!sql || !columnName || !schemaInfo) return null;
  return pickSourceTable(resolveKnownTables(sql, schemaInfo), columnName);
}

/**
 * Find the foreign key constraint that covers `columnName` on the given table.
 * Composite constraints are returned whole so callers can filter on every column.
 */
export function findForeignKeyForColumn(
  schemaInfo: SchemaInfo | null,
  source: { schema: string; table: string },
  columnName: string
): ForeignKeyReference | null {
  if (!schemaInfo) return null;

  const matching = schemaInfo.relationships.filter(
    (rel) =>
      eq(rel.source_schema_name, source.schema) &&
      eq(rel.source_table_name, source.table) &&
      eq(rel.source_column_name, columnName)
  );

  if (matching.length === 0) return null;

  // A column can participate in more than one FK; prefer the constraint with
  // the fewest columns (the most direct lookup).
  const byConstraint = new Map<string, SchemaRelationshipInfo[]>();
  for (const rel of schemaInfo.relationships) {
    if (!eq(rel.source_schema_name, source.schema) || !eq(rel.source_table_name, source.table)) continue;
    const existing = byConstraint.get(rel.constraint_name);
    if (existing) existing.push(rel);
    else byConstraint.set(rel.constraint_name, [rel]);
  }

  const constraintNames = Array.from(new Set(matching.map((rel) => rel.constraint_name)));
  let best: SchemaRelationshipInfo[] | null = null;
  for (const name of constraintNames) {
    const columns = byConstraint.get(name);
    if (!columns || columns.length === 0) continue;
    if (!best || columns.length < best.length) best = columns;
  }

  if (!best) return null;

  const ordered = [...best].sort((a, b) => a.ordinal_position - b.ordinal_position);
  const clicked = ordered.find((rel) => eq(rel.source_column_name, columnName)) ?? ordered[0];

  return {
    constraintName: clicked.constraint_name,
    sourceSchema: clicked.source_schema_name,
    sourceTable: clicked.source_table_name,
    targetSchema: clicked.target_schema_name,
    targetTable: clicked.target_table_name,
    targetColumn: clicked.target_column_name,
    columnPairs: ordered.map((rel) => ({
      sourceColumn: rel.source_column_name,
      targetColumn: rel.target_column_name,
    })),
    isVirtual: false,
  };
}

/** Find the user-defined reference for a column, if one is configured. */
export function findVirtualReferenceForColumn(
  virtualReferences: VirtualReference[] | null | undefined,
  source: { schema: string; table: string },
  columnName: string
): ForeignKeyReference | null {
  if (!virtualReferences || virtualReferences.length === 0) return null;

  const match = virtualReferences.find(
    (reference) =>
      eq(reference.source_schema, source.schema) &&
      eq(reference.source_table, source.table) &&
      eq(reference.source_column, columnName)
  );
  if (!match) return null;

  return {
    constraintName: null,
    sourceSchema: match.source_schema,
    sourceTable: match.source_table,
    targetSchema: match.target_schema,
    targetTable: match.target_table,
    targetColumn: match.target_column,
    columnPairs: [{ sourceColumn: match.source_column, targetColumn: match.target_column }],
    isVirtual: true,
    virtualReferenceId: match.id,
  };
}

/**
 * Resolve the reference (if any) for a single result column. A real foreign key
 * always wins over a user-defined one for the same column.
 */
export function findColumnReference(
  sql: string | null | undefined,
  columnName: string,
  schemaInfo: SchemaInfo | null,
  virtualReferences?: VirtualReference[] | null
): ForeignKeyReference | null {
  const source = resolveColumnSourceTable(sql, columnName, schemaInfo);
  if (!source) return null;
  return (
    findForeignKeyForColumn(schemaInfo, source, columnName) ??
    findVirtualReferenceForColumn(virtualReferences, source, columnName)
  );
}

/** Per-column resolution for a result set. */
export interface ColumnReferenceIndex {
  /** Owning table per column index — present even when there is no reference,
   *  so the UI can offer to define one. */
  sources: Map<number, { schema: string; table: string }>;
  /** Real or user-defined reference per column index. */
  references: Map<number, ForeignKeyReference>;
}

/**
 * Build the per-column reference index for a result set. The statement is parsed
 * once and duplicate column names reuse the first resolution, so this stays
 * cheap even for wide results.
 */
export function buildColumnReferenceIndex(
  sql: string | null | undefined,
  columns: Array<{ name: string }>,
  schemaInfo: SchemaInfo | null,
  virtualReferences?: VirtualReference[] | null
): ColumnReferenceIndex {
  const index: ColumnReferenceIndex = { sources: new Map(), references: new Map() };
  if (!sql || !schemaInfo || columns.length === 0) return index;

  const knownTables = resolveKnownTables(sql, schemaInfo);
  if (knownTables.length === 0) return index;

  const sourceCache = new Map<string, { schema: string; table: string } | null>();
  const referenceCache = new Map<string, ForeignKeyReference | null>();

  columns.forEach((column, position) => {
    const key = column.name.toLowerCase();

    let source = sourceCache.get(key);
    if (source === undefined) {
      source = pickSourceTable(knownTables, column.name);
      sourceCache.set(key, source);
    }
    if (!source) return;
    index.sources.set(position, source);

    let reference = referenceCache.get(key);
    if (reference === undefined) {
      reference =
        findForeignKeyForColumn(schemaInfo, source, column.name) ??
        findVirtualReferenceForColumn(virtualReferences, source, column.name);
      referenceCache.set(key, reference);
    }
    if (reference) index.references.set(position, reference);
  });

  return index;
}
