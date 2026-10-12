/**
 * SQL identifier quoting helpers (frontend).
 * Mirrors src-tauri/src/db/mssql/ident.rs and src-tauri/src/db/postgres/ident.rs.
 */
import type { DriverKind } from '../../types';

/** Bracket-quote a T-SQL identifier, escaping `]` → `]]`. */
export function quoteIdent(name: string): string {
  return '[' + name.replace(/\]/g, ']]') + ']';
}

export function unwrapIdent(tok: string): string {
  if (tok.startsWith('[') && tok.endsWith(']')) {
    return tok.slice(1, -1).replace(/\]\]/g, ']');
  }
  return tok;
}

/**
 * PostgreSQL words that cannot appear bare as an identifier. Subset of the
 * reserved list; anything ambiguous gets double-quoted, which is always safe.
 */
const PG_RESERVED: Record<string, true> = {
  all: true, analyse: true, analyze: true, and: true, any: true, array: true,
  as: true, asc: true, asymmetric: true, authorization: true, between: true,
  bigint: true, binary: true, bit: true, boolean: true, both: true, case: true,
  cast: true, char: true, character: true, check: true, collate: true,
  collation: true, column: true, constraint: true, create: true,
  current_catalog: true, current_date: true, current_role: true,
  current_schema: true, current_time: true, current_timestamp: true,
  current_user: true, default: true, deferrable: true, desc: true, distinct: true,
  do: true, else: true, end: true, except: true, exists: true, false: true,
  fetch: true, float: true, for: true, foreign: true, from: true, grant: true,
  group: true, having: true, in: true, index: true, initially: true, inner: true,
  insert: true, intersect: true, into: true, is: true, join: true, key: true,
  lateral: true, leading: true, left: true, like: true, limit: true,
  localtime: true, localtimestamp: true, natural: true, not: true, null: true,
  offset: true, on: true, only: true, or: true, order: true, outer: true,
  overlaps: true, placing: true, primary: true, references: true, returning: true,
  right: true, select: true, session_user: true, set: true, similar: true,
  some: true, table: true, then: true, to: true, trailing: true, true: true,
  union: true, unique: true, update: true, user: true, using: true, values: true,
  variadic: true, view: true, when: true, where: true, with: true,
};

/** Unquoted PostgreSQL identifiers: lowercase start, `[a-z0-9_$]`, not reserved. */
export function isSafePostgresBareIdent(name: string): boolean {
  return /^[a-z_][a-z0-9_$]*$/.test(name) && !PG_RESERVED[name];
}

/**
 * Quote an identifier for the active driver:
 * - mssql: `[name]` with `]]` escaping
 * - postgres: bare when unambiguous, else `"name"` with `""` escaping
 */
export function quoteSqlIdent(name: string, driver: DriverKind): string {
  if (driver === 'postgres') {
    return isSafePostgresBareIdent(name) ? name : `"${name.replace(/"/g, '""')}"`;
  }
  return quoteIdent(name);
}

/** Quote a `schema.name` pair for the active driver. */
export function quoteQualifiedSqlIdent(schema: string, name: string, driver: DriverKind): string {
  return `${quoteSqlIdent(schema, driver)}.${quoteSqlIdent(name, driver)}`;
}
