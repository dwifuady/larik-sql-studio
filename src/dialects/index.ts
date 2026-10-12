import type { DriverKind } from '../types';
import { mssqlDialect } from './mssql';
import { postgresDialect } from './postgres';
import type { SqlDialect, SqlDriverKind } from './types';

export type { SqlDialect, SqlDialectFeatures, SqlDriverKind } from './types';
export { mssqlDialect, postgresDialect };

/** Dialect used when none applies (pre-driver spaces default to SQL Server). */
export const DEFAULT_SQL_DIALECT: SqlDialect = mssqlDialect;

const DIALECTS: Record<string, SqlDialect> = {
  mssql: mssqlDialect,
  postgres: postgresDialect,
};

/** Drivers that have a SQL dialect, in display order. */
export const SQL_DRIVERS: readonly SqlDriverKind[] = Object.values(DIALECTS).map(dialect => dialect.driver);

/** Every Monaco language a SQL dialect uses (for language-scoped registrations). */
export const SQL_MONACO_LANGUAGES: readonly string[] = [
  ...new Set(Object.values(DIALECTS).map(dialect => dialect.monacoLanguage)),
];

/**
 * The SQL dialect for a driver, or null for engines without one (MongoDB's
 * editor uses its own language and tooling).
 */
export function getSqlDialect(driver: DriverKind): SqlDialect | null {
  return DIALECTS[driver] ?? null;
}
