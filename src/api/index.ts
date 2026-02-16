// Tauri IPC API wrapper functions
import { invoke } from '@tauri-apps/api/core';
import { save, open } from '@tauri-apps/plugin-dialog';
import type { Space, Tab, ConnectionInfo, QueryResult, QueryInfo, CreateSpaceInput, UpdateSpaceInput, SchemaInfo, SchemaColumnInfo } from '../types';
import type { TabFolder } from '../types';
import type { Snippet, CreateSnippetInput, UpdateSnippetInput } from '../types';
import type { ArchivedTab, ArchiveSearchResult, AutoArchiveSettings, AppSettings } from '../types';

// ============================================================================
// Space API (with integrated connection - 1:1 model)
// ============================================================================

export async function createSpace(input: CreateSpaceInput): Promise<Space> {
  return invoke<Space>('create_space', {
    name: input.name,
    color: input.color,
    icon: input.icon,
    connectionHost: input.connection_host,
    connectionPort: input.connection_port,
    connectionDatabase: input.connection_database,
    connectionUsername: input.connection_username,
    connectionPassword: input.connection_password,
    connectionTrustCert: input.connection_trust_cert,
    connectionEncrypt: input.connection_encrypt,
  });
}

export async function getSpaces(): Promise<Space[]> {
  return invoke<Space[]>('get_spaces');
}

export async function getSpace(id: string): Promise<Space | null> {
  return invoke<Space | null>('get_space', { id });
}

export async function updateSpace(id: string, input: UpdateSpaceInput): Promise<Space | null> {
  return invoke<Space | null>('update_space', {
    id,
    name: input.name,
    color: input.color,
    icon: input.icon,
    sortOrder: input.sort_order,
    connectionHost: input.connection_host,
    connectionPort: input.connection_port,
    connectionDatabase: input.connection_database,
    connectionUsername: input.connection_username,
    connectionPassword: input.connection_password,
    connectionTrustCert: input.connection_trust_cert,
    connectionEncrypt: input.connection_encrypt,
  });
}

export async function deleteSpace(id: string): Promise<boolean> {
  return invoke<boolean>('delete_space', { id });
}

export async function reorderSpaces(spaceIds: string[]): Promise<void> {
  return invoke<void>('reorder_spaces', { spaceIds });
}

export async function updateSpaceLastActiveTab(spaceId: string, tabId: string | null): Promise<void> {
  return invoke<void>('update_space_last_active_tab', { spaceId, tabId });
}

// ============================================================================
// Space Connection API (1:1 model - T018)
// ============================================================================

/** Connect to a space's database (uses space ID as connection ID) */
export async function connectToSpace(spaceId: string): Promise<boolean> {
  return invoke<boolean>('connect_to_space', { spaceId });
}

/** Disconnect from a space's database */
export async function disconnectFromSpace(spaceId: string): Promise<boolean> {
  return invoke<boolean>('disconnect_from_space', { spaceId });
}

/** Get connection status for a space */
export async function getSpaceConnectionStatus(spaceId: string): Promise<ConnectionInfo | null> {
  return invoke<ConnectionInfo | null>('get_space_connection_status', { spaceId });
}

/** Get list of databases from a space's server connection */
export async function getSpaceDatabases(spaceId: string): Promise<string[]> {
  return invoke<string[]>('get_space_databases', { spaceId });
}

// ============================================================================
// Tab API
// ============================================================================

export async function createTab(
  spaceId: string,
  title: string,
  tabType: string = 'query',
  content?: string | null,
  metadata?: string | null,
  database?: string | null
): Promise<Tab> {
  return invoke<Tab>('create_tab', { spaceId, title, tabType, content, metadata, database });
}

export async function getTabsBySpace(spaceId: string): Promise<Tab[]> {
  return invoke<Tab[]>('get_tabs_by_space', { spaceId });
}

export async function getTab(id: string): Promise<Tab | null> {
  return invoke<Tab | null>('get_tab', { id });
}

export async function searchTabs(query: string): Promise<Tab[]> {
  return invoke<Tab[]>('search_tabs', { query });
}

export async function updateTab(
  id: string,
  title?: string | null,
  content?: string | null,
  metadata?: string | null,
  database?: string | null,
  sortOrder?: number | null
): Promise<Tab | null> {
  return invoke<Tab | null>('update_tab', { id, title, content, metadata, database, sortOrder });
}

/** Update just the database selection for a tab */
export async function updateTabDatabase(id: string, database: string | null): Promise<boolean> {
  return invoke<boolean>('update_tab_database', { id, database });
}

export async function autosaveTabContent(id: string, content: string): Promise<boolean> {
  return invoke<boolean>('autosave_tab_content', { id, content });
}

export async function toggleTabPinned(id: string): Promise<Tab | null> {
  return invoke<Tab | null>('toggle_tab_pinned', { id });
}

export async function deleteTab(id: string): Promise<boolean> {
  return invoke<boolean>('delete_tab', { id });
}

export async function reorderTabs(spaceId: string, tabIds: string[]): Promise<void> {
  return invoke<void>('reorder_tabs', { spaceId, tabIds });
}

export async function moveTabToSpace(tabId: string, newSpaceId: string): Promise<Tab | null> {
  return invoke<Tab | null>('move_tab_to_space', { tabId, newSpaceId });
}

// ============================================================================
// Folder API - Arc Browser-style tab organization
// ============================================================================

export async function createFolder(spaceId: string, name: string): Promise<TabFolder> {
  return invoke<TabFolder>('create_folder', { spaceId, name });
}

export async function getFoldersBySpace(spaceId: string): Promise<TabFolder[]> {
  return invoke<TabFolder[]>('get_folders_by_space', { spaceId });
}

export async function updateFolder(
  id: string,
  name?: string | null,
  isExpanded?: boolean | null,
  sortOrder?: number | null
): Promise<TabFolder | null> {
  return invoke<TabFolder | null>('update_folder', { id, name, isExpanded, sortOrder });
}

export async function deleteFolder(id: string): Promise<boolean> {
  return invoke<boolean>('delete_folder', { id });
}

export async function addTabToFolder(tabId: string, folderId: string): Promise<boolean> {
  return invoke<boolean>('add_tab_to_folder', { tabId, folderId });
}

export async function removeTabFromFolder(tabId: string): Promise<boolean> {
  return invoke<boolean>('remove_tab_from_folder', { tabId });
}

export async function reorderFolders(spaceId: string, folderIds: string[]): Promise<void> {
  return invoke<void>('reorder_folders', { spaceId, folderIds });
}

export async function createFolderFromTabs(spaceId: string, name: string, tabIds: string[]): Promise<TabFolder> {
  return invoke<TabFolder>('create_folder_from_tabs', { spaceId, name, tabIds });
}

// ============================================================================
// Connection API (T018)
// ============================================================================

export async function createConnection(
  name: string,
  host: string,
  port: number,
  database: string,
  username: string,
  password: string,
  spaceId?: string | null,
  trustCertificate?: boolean,
  encrypt?: boolean
): Promise<ConnectionInfo> {
  return invoke<ConnectionInfo>('create_connection', {
    name,
    host,
    port,
    database,
    username,
    password,
    spaceId,
    trustCertificate,
    encrypt,
  });
}

export async function testConnection(
  host: string,
  port: number,
  database: string,
  username: string,
  password: string,
  trustCertificate?: boolean,
  encrypt?: boolean
): Promise<boolean> {
  return invoke<boolean>('test_connection', {
    host,
    port,
    database,
    username,
    password,
    trustCertificate,
    encrypt,
  });
}

export async function getConnections(): Promise<ConnectionInfo[]> {
  return invoke<ConnectionInfo[]>('get_connections');
}

export async function getConnectionsBySpace(spaceId: string): Promise<ConnectionInfo[]> {
  return invoke<ConnectionInfo[]>('get_connections_by_space', { spaceId });
}

export async function getConnection(id: string): Promise<ConnectionInfo | null> {
  return invoke<ConnectionInfo | null>('get_connection', { id });
}

export async function updateConnection(
  id: string,
  name?: string | null,
  host?: string | null,
  port?: number | null,
  database?: string | null,
  username?: string | null,
  password?: string | null,
  spaceId?: string | null,
  trustCertificate?: boolean,
  encrypt?: boolean
): Promise<ConnectionInfo> {
  return invoke<ConnectionInfo>('update_connection', {
    id,
    name,
    host,
    port,
    database,
    username,
    password,
    spaceId,
    trustCertificate,
    encrypt,
  });
}

export async function deleteConnection(id: string): Promise<boolean> {
  return invoke<boolean>('delete_connection', { id });
}

export async function connectDatabase(connectionId: string): Promise<boolean> {
  return invoke<boolean>('connect_database', { connectionId });
}

export async function disconnectDatabase(connectionId: string): Promise<boolean> {
  return invoke<boolean>('disconnect_database', { connectionId });
}

export async function getConnectionDatabases(connectionId: string): Promise<string[]> {
  return invoke<string[]>('get_connection_databases', { connectionId });
}

export async function checkConnectionHealth(connectionId: string): Promise<boolean> {
  return invoke<boolean>('check_connection_health', { connectionId });
}

// ============================================================================
// Query Execution API (T019)
// ============================================================================

export async function executeQuery(
  connectionId: string,
  query: string,
  database?: string | null,
  selectedText?: string | null
): Promise<QueryResult[]> {
  return invoke<QueryResult[]>('execute_query', { connectionId, query, database, selectedText });
}

export async function cancelQuery(queryId: string): Promise<boolean> {
  return invoke<boolean>('cancel_query', { queryId });
}

export async function cancelQueriesForConnection(connectionId: string): Promise<number> {
  return invoke<number>('cancel_queries_for_connection', { connectionId });
}

export async function getQueryStatus(queryId: string): Promise<QueryInfo | null> {
  return invoke<QueryInfo | null>('get_query_status', { queryId });
}

// ============================================================================
// Schema Metadata API (T025)
// ============================================================================

/** Get complete schema information for a database (tables, views, columns, routines) */
export async function getSchemaInfo(
  connectionId: string,
  database: string,
  schemaFilter?: string | null,
  forceRefresh?: boolean
): Promise<SchemaInfo> {
  return invoke<SchemaInfo>('get_schema_info', {
    connectionId,
    database,
    schemaFilter,
    forceRefresh,
  });
}

/** Get columns for a specific table */
export async function getTableColumns(
  connectionId: string,
  database: string,
  schemaName: string,
  tableName: string
): Promise<SchemaColumnInfo[]> {
  return invoke<SchemaColumnInfo[]>('get_table_columns', {
    connectionId,
    database,
    schemaName,
    tableName,
  });
}

/** Force refresh schema cache for a connection/database */
export async function refreshSchema(connectionId: string, database?: string | null): Promise<void> {
  return invoke<void>('refresh_schema', { connectionId, database });
}

// ============================================================================
// Export API (T034, T035, T036)
// ============================================================================

import type { ExportOptions, ExportProgress, ColumnInfo, CellValue } from '../types';

/** Export query results to CSV file */
export async function exportToCsv(
  filePath: string,
  columns: ColumnInfo[],
  rows: CellValue[][],
  options?: ExportOptions
): Promise<ExportProgress> {
  return invoke<ExportProgress>('export_to_csv', {
    filePath,
    columns,
    rows,
    options,
  });
}

/** Export query results to JSON file */
export async function exportToJson(
  filePath: string,
  columns: ColumnInfo[],
  rows: CellValue[][],
  options?: ExportOptions
): Promise<ExportProgress> {
  return invoke<ExportProgress>('export_to_json', {
    filePath,
    columns,
    rows,
    options,
  });
}

/** Export query results to string (CSV or JSON) for clipboard copy */
export async function exportToString(
  format: 'csv' | 'json',
  columns: ColumnInfo[],
  rows: CellValue[][],
  options?: ExportOptions
): Promise<string> {
  return invoke<string>('export_to_string', {
    format,
    columns,
    rows,
    options,
  });
}

/** Cancel an ongoing export operation */
export async function cancelExport(exportId: string): Promise<boolean> {
  return invoke<boolean>('cancel_export', { exportId });
}

// ============================================================================
// SQL File Import/Export API
// ============================================================================

/** Sanitize filename for SQL export */
export function sanitizeFilename(title: string): string {
  return title
    .replace(/[/\\:*?"<>|]/g, '-')
    .replace(/\s+/g, ' ')
    .trim()
    .substring(0, 200)
    .concat('.sql');
}

/** Show save dialog for SQL file */
export async function saveSqlFileDialog(defaultName: string): Promise<string | null> {
  return save({
    defaultPath: defaultName,
    filters: [
      { name: 'SQL Files', extensions: ['sql'] },
      { name: 'All Files', extensions: ['*'] }
    ]
  });
}

/** Show open dialog for SQL file */
export async function openSqlFileDialog(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    filters: [
      { name: 'SQL Files', extensions: ['sql'] },
      { name: 'All Files', extensions: ['*'] }
    ]
  });
  return Array.isArray(selected) ? selected[0] : selected;
}

/** Export tab content to SQL file */
export async function exportTabAsSql(tabId: string, filePath: string, content?: string): Promise<void> {
  return invoke<void>('export_tab_as_sql', { tabId, filePath, content });
}

/** Import SQL file as a new tab */
export async function importSqlFileAsTab(
  spaceId: string,
  filePath: string,
  title?: string
): Promise<Tab> {
  return invoke<Tab>('import_sql_file_as_tab', { spaceId, filePath, title });
}

// ============================================================================
// Snippet API (T046)
// ============================================================================

/** Get all snippets */
export async function getSnippets(): Promise<Snippet[]> {
  return invoke<Snippet[]>('get_snippets');
}

/** Get only enabled snippets (for editor use) */
export async function getEnabledSnippets(): Promise<Snippet[]> {
  return invoke<Snippet[]>('get_enabled_snippets');
}

/** Get a single snippet by ID */
export async function getSnippet(id: string): Promise<Snippet | null> {
  return invoke<Snippet | null>('get_snippet', { id });
}

/** Get a snippet by trigger text */
export async function getSnippetByTrigger(trigger: string): Promise<Snippet | null> {
  return invoke<Snippet | null>('get_snippet_by_trigger', { trigger });
}

/** Create a new user-defined snippet */
export async function createSnippet(input: CreateSnippetInput): Promise<Snippet> {
  return invoke<Snippet>('create_snippet', {
    trigger: input.trigger,
    name: input.name,
    content: input.content,
    description: input.description,
    category: input.category,
  });
}

/** Update an existing snippet */
export async function updateSnippet(id: string, input: UpdateSnippetInput): Promise<Snippet | null> {
  return invoke<Snippet | null>('update_snippet', {
    id,
    trigger: input.trigger,
    name: input.name,
    content: input.content,
    description: input.description,
    category: input.category,
    enabled: input.enabled,
  });
}

/** Delete a snippet (only user-defined snippets can be deleted) */
export async function deleteSnippet(id: string): Promise<boolean> {
  return invoke<boolean>('delete_snippet', { id });
}

/** Reset a builtin snippet to its default content */
export async function resetBuiltinSnippet(id: string): Promise<Snippet | null> {
  return invoke<Snippet | null>('reset_builtin_snippet', { id });
}

/** Import snippets from external source (bulk import) */
export async function importSnippets(snippets: CreateSnippetInput[]): Promise<number> {
  return invoke<number>('import_snippets', { snippets });
}

/** Show open dialog for DBeaver templates XML file */
export async function openDbeaverTemplatesDialog(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    filters: [
      { name: 'XML Files', extensions: ['xml'] },
      { name: 'All Files', extensions: ['*'] }
    ]
  });
  return Array.isArray(selected) ? selected[0] : selected;
}

// ============================================================================
// Archive/History Commands - Arc Browser-style tab history
// ============================================================================

/** Archive a tab (move from active to archived) */
export async function archiveTab(tabId: string): Promise<ArchivedTab> {
  return invoke<ArchivedTab>('archive_tab', { tabId });
}

/** Restore an archived tab back to active tabs */
export async function restoreArchivedTab(
  archivedId: string,
  targetSpaceId?: string | null
): Promise<Tab> {
  return invoke<Tab>('restore_archived_tab', { archivedId, targetSpaceId });
}

/** Search archived tabs using FTS5 full-text search */
export async function searchArchivedTabs(
  query: string,
  spaceId?: string | null,
  limit?: number
): Promise<ArchiveSearchResult[]> {
  return invoke<ArchiveSearchResult[]>('search_archived_tabs', { query, spaceId, limit });
}

/** Get archived tabs with optional space filter and pagination */
export async function getArchivedTabs(
  spaceId?: string | null,
  limit?: number,
  offset?: number
): Promise<ArchivedTab[]> {
  return invoke<ArchivedTab[]>('get_archived_tabs', { spaceId, limit, offset });
}

/** Get count of archived tabs with optional space filter */
export async function getArchivedTabsCount(spaceId?: string | null): Promise<number> {
  return invoke<number>('get_archived_tabs_count', { spaceId });
}

/** Permanently delete an archived tab */
export async function deleteArchivedTab(archivedId: string): Promise<boolean> {
  return invoke<boolean>('delete_archived_tab', { archivedId });
}

/** Update last_accessed_at timestamp for a tab (activity tracking) */
export async function touchTab(tabId: string): Promise<boolean> {
  return invoke<boolean>('touch_tab', { tabId });
}

/** Get auto-archive settings */
export async function getAutoArchiveSettings(): Promise<AutoArchiveSettings> {
  return invoke<AutoArchiveSettings>('get_auto_archive_settings');
}

/** Update auto-archive settings */
export async function updateAutoArchiveSettings(
  enabled: boolean,
  daysInactive: number
): Promise<void> {
  return invoke<void>('update_auto_archive_settings', { enabled, daysInactive });
}

// ============================================================================
// App Settings API
// ============================================================================

/** Get app settings (validation, last opened workspace/tab) */
export async function getAppSettings(): Promise<AppSettings> {
  return invoke<AppSettings>('get_app_settings');
}

/** Update app settings */
export async function updateAppSettings(
  validationEnabled: boolean,
  lastSpaceId: string | null,
  lastTabId: string | null,
  enableStickyNotes: boolean,
  maxResultRows: number
): Promise<void> {
  return invoke<void>('update_app_settings', {
    validationEnabled,
    lastSpaceId,
    lastTabId,
    enableStickyNotes,
    maxResultRows
  });
}
