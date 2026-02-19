// TypeScript interfaces matching Rust backend models

/** Tab type enum matching backend */
export type TabType = 'query' | 'results' | 'schema' | 'settings';

/** A Space represents a work environment containing related tabs and 1:1 connection */
export interface Space {
  id: string;
  name: string;
  color: string | null;
  icon: string | null;
  created_at: string;
  updated_at: string;
  sort_order: number;
  // Connection fields (1:1 model)
  connection_host: string | null;
  connection_port: number | null;
  connection_database: string | null;
  connection_username: string | null;
  // Note: password is not returned from backend for security
  connection_trust_cert: boolean | null;
  connection_encrypt: boolean | null;
  last_active_tab_id: string | null;
}

/** Check if a space has a connection configured */
export function spaceHasConnection(space: Space): boolean {
  return !!(space.connection_host && space.connection_database);
}

/** Input for creating a new space with optional connection */
export interface CreateSpaceInput {
  name: string;
  color?: string | null;
  icon?: string | null;
  // Connection fields
  connection_host?: string | null;
  connection_port?: number | null;
  connection_database?: string | null;
  connection_username?: string | null;
  connection_password?: string | null;
  connection_trust_cert?: boolean | null;
  connection_encrypt?: boolean | null;
}

/** Input for updating an existing space */
export interface UpdateSpaceInput {
  name?: string | null;
  color?: string | null;
  icon?: string | null;
  sort_order?: number | null;
  // Connection fields
  connection_host?: string | null;
  connection_port?: number | null;
  connection_database?: string | null;
  connection_username?: string | null;
  connection_password?: string | null;
  connection_trust_cert?: boolean | null;
  connection_encrypt?: boolean | null;
}

/** A Tab represents a tab within a space (can be pinned or unpinned) */
export interface Tab {
  id: string;
  space_id: string;
  title: string;
  tab_type: TabType;
  content: string | null;
  metadata: string | null;
  database: string | null;  // Per-tab database selection
  folder_id: string | null;  // ID of folder this tab belongs to (Arc Browser-style)
  is_pinned: boolean;
  created_at: string;
  updated_at: string;
  sort_order: number;
}

/** Input for creating a new tab */
export interface CreateTabInput {
  space_id: string;
  title: string;
  tab_type: TabType;
  content?: string | null;
  metadata?: string | null;
  database?: string | null;
}

/** Input for updating an existing tab */
export interface UpdateTabInput {
  title?: string | null;
  content?: string | null;
  metadata?: string | null;
  database?: string | null;
  sort_order?: number | null;
}

// ============================================================================
// Folder Types - Arc Browser-style tab organization
// ============================================================================

/** A folder for organizing pinned tabs */
export interface TabFolder {
  id: string;
  space_id: string;
  name: string;
  is_expanded: boolean;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

/** Input for creating a new folder */
export interface CreateFolderInput {
  space_id: string;
  name: string;
}

/** Input for updating an existing folder */
export interface UpdateFolderInput {
  name?: string | null;
  is_expanded?: boolean | null;
  sort_order?: number | null;
}

/** Folder with its child tabs for rendering */
export interface FolderWithTabs {
  folder: TabFolder;
  tabs: Tab[];
}

/** Grouped pinned tabs for rendering sidebar */
export interface PinnedTabsGrouped {
  ungrouped: Tab[];
  folders: FolderWithTabs[];
}

// ============================================================================
// Connection Types (T018, T020)
// ============================================================================

/** Database connection configuration info (without password) */
export interface ConnectionInfo {
  id: string;
  name: string;
  host: string;
  port: number;
  database: string;
  username: string;
  trust_certificate: boolean;
  encrypt: boolean;
  space_id: string | null;
  is_connected: boolean;
}

/** Input for creating a new connection */
export interface CreateConnectionInput {
  name: string;
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
  space_id?: string | null;
  trust_certificate?: boolean;
  encrypt?: boolean;
}

/** Input for testing a connection */
export interface TestConnectionInput {
  host: string;
  port: number;
  database: string;
  username: string;
  password: string;
  trust_certificate?: boolean;
  encrypt?: boolean;
}

/** Input for updating a connection */
export interface UpdateConnectionInput {
  name?: string | null;
  host?: string | null;
  port?: number | null;
  database?: string | null;
  username?: string | null;
  password?: string | null;
  space_id?: string | null;
  trust_certificate?: boolean | null;
  encrypt?: boolean | null;
}

// ============================================================================
// Query Types (T019)
// ============================================================================

/** Cell value in query results */
export type CellValue = null | boolean | number | string | number[];

/** Column information from query results */
export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
}

/** Query result from executed SQL */
export interface QueryResult {
  query_id: string;
  columns: ColumnInfo[];
  rows: CellValue[][];
  row_count: number;
  execution_time_ms: number;
  error: string | null;
  is_complete: boolean;
  is_selection: boolean; // Indicates if this was executed from selected text
  statement_index: number | null; // Index in batch execution (null for single query)
  statement_text: string | null; // The actual SQL text executed (useful for batch)
  displayId?: number; // Stable ID for display (e.g., Result 1, Result 2)
}

/** Status of a query */
export type QueryStatus = 'Pending' | 'Running' | 'Completed' | 'Cancelled' | 'Error';

/** Information about a running/completed query */
export interface QueryInfo {
  query_id: string;
  connection_id: string;
  query: string;
  status: QueryStatus;
  started_at: string;
  rows_fetched: number;
}

// ============================================================================
// Schema Types (T024, T025, T026)
// ============================================================================

/** Detailed column information from schema metadata */
export interface SchemaColumnInfo {
  name: string;
  data_type: string;
  max_length: number | null;
  precision: number | null;
  scale: number | null;
  is_nullable: boolean;
  is_primary_key: boolean;
  is_identity: boolean;
  column_default: string | null;
  ordinal_position: number;
}

/** Table or view information with columns */
export interface TableInfo {
  schema_name: string;
  table_name: string;
  table_type: string; // "BASE TABLE" or "VIEW"
  columns: SchemaColumnInfo[];
}

/** Parameter information for stored procedures/functions */
export interface ParameterInfo {
  name: string;
  data_type: string;
  max_length: number | null;
  precision: number | null;
  scale: number | null;
  parameter_mode: string; // "IN", "OUT", "INOUT"
  ordinal_position: number;
  has_default: boolean;
}

/** Stored procedure or function information */
export interface RoutineInfo {
  schema_name: string;
  routine_name: string;
  routine_type: string; // "PROCEDURE" or "FUNCTION"
  return_type: string | null;
  parameters: ParameterInfo[];
}

/** Complete schema information for a database */
export interface SchemaInfo {
  database_name: string;
  schemas: string[];
  tables: TableInfo[];
  routines: RoutineInfo[];
  fetched_at: string;
}

/** Get fully qualified name for a table */
export function getFullTableName(table: TableInfo): string {
  return `[${table.schema_name}].[${table.table_name}]`;
}

/** Get display data type including size info */
export function getDisplayDataType(column: SchemaColumnInfo): string {
  const { data_type, max_length, precision, scale } = column;

  // Types with length
  if (['varchar', 'nvarchar', 'char', 'nchar', 'varbinary', 'binary'].includes(data_type.toLowerCase())) {
    if (max_length === -1) {
      return `${data_type}(max)`;
    }
    return max_length ? `${data_type}(${max_length})` : data_type;
  }

  // Types with precision and scale
  if (['decimal', 'numeric'].includes(data_type.toLowerCase())) {
    return precision !== null ? `${data_type}(${precision},${scale ?? 0})` : data_type;
  }

  return data_type;
}

// ============================================================================
// Export Types (T034, T035, T036)
// ============================================================================

/** Export format options */
export type ExportFormat = 'csv' | 'json';

/** Export options for customizing output */
export interface ExportOptions {
  /** Include column headers (CSV) or use them as JSON keys */
  include_headers?: boolean;
  /** Pretty print JSON output */
  pretty_print?: boolean;
  /** Delimiter for CSV (default: comma) */
  delimiter?: string;
  /** Quote character for CSV (default: double quote) */
  quote_char?: string;
  /** Include NULL values as "NULL" string or empty */
  null_as_string?: boolean;
  /** Maximum rows to export (undefined = all rows) */
  max_rows?: number;
}

/** Export progress information for UI updates */
export interface ExportProgress {
  rows_exported: number;
  total_rows: number;
  bytes_written: number;
  is_complete: boolean;
  error: string | null;
}

// ============================================================================
// Snippet Types (T046)
// ============================================================================

/** A SQL code snippet with trigger text and expansion */
export interface Snippet {
  id: string;
  /** The short trigger text (e.g., "sel", "ssf") */
  trigger: string;
  /** Display name for the snippet */
  name: string;
  /** The expanded SQL content with optional ${cursor} placeholder */
  content: string;
  /** Optional description */
  description: string | null;
  /** Whether this is a built-in snippet or user-defined */
  is_builtin: boolean;
  /** Whether the snippet is enabled */
  enabled: boolean;
  /** Category for grouping (e.g., "Select", "Insert", "DDL") */
  category: string | null;
  created_at: string;
  updated_at: string;
}

/** Input for creating a new snippet */
export interface CreateSnippetInput {
  trigger: string;
  name: string;
  content: string;
  description?: string | null;
  category?: string | null;
}

/** Input for updating an existing snippet */
export interface UpdateSnippetInput {
  trigger?: string | null;
  name?: string | null;
  content?: string | null;
  description?: string | null;
  category?: string | null;
  enabled?: boolean | null;
}

/** Parsed snippet from DBeaver XML format */
export interface DbeaverSnippet {
  name: string;
  content: string;
  description: string;
  context: string;
}

// ============================================================================
// Archive/History Types - Arc Browser-style tab history
// ============================================================================

/** An archived tab stored in history */
export interface ArchivedTab {
  id: string;
  original_tab_id: string;
  space_id: string | null;
  space_name: string;
  title: string;
  tab_type: TabType;
  content: string | null;
  metadata: string | null;
  database: string | null;
  was_pinned: boolean;
  created_at: string;
  updated_at: string;
  last_accessed_at: string;
  archived_at: string;
}

/** Archive search result with FTS5 relevance and snippets */
export interface ArchiveSearchResult {
  archived_tab: ArchivedTab;
  rank: number;
  snippet_title: string | null;
  snippet_content: string | null;
}

/** Auto-archive settings */
export interface AutoArchiveSettings {
  enabled: boolean;
  days_inactive: number;
}

/** App settings (validation, last opened workspace/tab) */
export interface AppSettings {
  validation_enabled: boolean;
  last_space_id: string | null;
  last_tab_id: string | null;
  enable_sticky_notes: boolean;
  max_result_rows: number;
}
