/**
 * SQL dialects: the frontend counterpart of the backend `DatabaseDriver`.
 *
 * Everything the editor, explorer and peek need to know about a SQL engine's
 * syntax lives behind this interface, so features ask the active dialect
 * instead of branching on the driver. MongoDB is not a SQL dialect: its
 * editor uses its own Monaco language and tooling (`utils/mongo/*`).
 */
import type { ParameterInfo, TableInfo } from '../types';

export type SqlDriverKind = 'mssql' | 'postgres';

/** Capabilities that only some engines support (or that the UI only implements for some). */
export interface SqlDialectFeatures {
  /** `USE database` statements switch databases (completion after USE). */
  useDatabase: boolean;
  /** `DECLARE @var` local variables (completion of declared variables). */
  localVariables: boolean;
  /** `GO` on its own line separates batches. */
  batchSeparator: boolean;
  /** Schema/column checks in the validator (they walk T-SQL AST shapes). */
  semanticValidation: boolean;
  /** Inline result-grid editing (generated UPDATE/INSERT/DELETE). */
  gridEdit: boolean;
  /** Foreign-key navigation from result cells (generated lookup queries). */
  foreignKeyNavigation: boolean;
}

export interface SqlDialect {
  readonly driver: SqlDriverKind;
  /** Monaco language id: drives highlighting; SQL completion providers register for it. */
  readonly monacoLanguage: string;
  /** node-sql-parser `database` options, tried in order. */
  readonly parserDatabases: readonly string[];
  /** sql-formatter `language`. */
  readonly formatterLanguage: 'tsql' | 'postgresql';
  /** Identifier quoting style (bracket `[x]` vs double-quote `"x"`). */
  readonly identifierQuote: 'bracket' | 'double';
  /** Schema assumed for unqualified object names. */
  readonly defaultSchema: string;
  /** Schema names recognised as a `schema.` qualifier even when not in the loaded schema. */
  readonly systemSchemas: readonly string[];
  /** Keywords offered by keyword completion. */
  readonly keywords: readonly string[];
  /** Built-in functions offered in expression contexts. */
  readonly functions: readonly string[];
  /** Keywords after which a table name is expected. */
  readonly tableContextKeywords: readonly string[];
  /** Keywords after which a table alias is worth generating. */
  readonly tableAliasKeywords: readonly string[];
  /** Keywords after which a procedure call is expected. */
  readonly routineContextKeywords: readonly string[];
  readonly features: SqlDialectFeatures;

  /** Quote an identifier only as much as the engine requires. */
  quoteIdent(name: string): string;
  /** Quote a `schema.name` pair; the default schema is kept explicit. */
  quoteQualified(schema: string, name: string): string;
  /** Name to insert for an object: bare-qualified only outside the default schema. */
  objectReference(schema: string, name: string): string;
  /** A single-quoted string literal. */
  quoteString(value: string): string;
  /** Split a script into statements (trimmed, without terminators). */
  splitStatements(sql: string): string[];

  /** First `limit` rows of a table or view. */
  selectTopQuery(schema: string, table: string, limit: number): string;
  /** Query returning one row, one column: the CREATE text of a view or routine. */
  objectDefinitionQuery(kind: 'view' | 'routine', schema: string, name: string): string;
  /** A readable CREATE TABLE script synthesized from schema metadata. */
  createTableScript(table: TableInfo): string;
  /**
   * Monaco snippet text inserted after EXEC/CALL to run a procedure, with
   * input parameters as tab stops: `p @a = ${1:NULL}` (T-SQL) or
   * `p(${1:NULL})` (PostgreSQL).
   */
  procedureCallSnippet(schema: string, name: string, parameters: ParameterInfo[]): string;
  /** Monaco snippet text calling a function in an expression or FROM clause. */
  functionCallSnippet(schema: string, name: string, parameters: ParameterInfo[]): string;
}
