// Arc-style query editor component with Monaco editor
import { useRef, useCallback, useEffect, useState, memo } from 'react';
import Editor, { OnMount, OnChange } from '@monaco-editor/react';
import type { editor, languages, IDisposable, IRange } from 'monaco-editor';
import { useAppStore } from '../store'; // Consolidated import if possible, but just ensuring it's there
import { DragDropContext, Droppable, Draggable, DropResult } from '@hello-pangea/dnd';
import type { Tab, SchemaInfo, SchemaColumnInfo, ParameterInfo } from '../types';
import { spaceHasConnection, getDisplayDataType } from '../types';
import { ResultsGrid } from './ResultsGrid';
import { formatSqlWithIndentation } from '../utils/sqlFormatter';
import { useSqlValidation } from '../hooks/useSqlValidation';
import { extractStatementAtCursor, findCurrentSqlBlockFallback } from '../utils/queryExtractor';
import {
  parseTableAliases as parseTableAliasesAST,
  getUsedAliases as getUsedAliasesAST,
  extractReferencedTables as extractReferencedTablesAST,
  getCompletionContext as getCompletionContextAST,
  type CompletionContext,
} from '../utils/sqlAstExtractor';
import { useStickyNotes, EVENT_ADD_STICKY_NOTE } from '../hooks/useStickyNotes';
import { extractAllStatements } from '../utils/queryExtractor';
import { extractNotes } from '../utils/noteManager';
import { ContextMenu, ContextMenuItem } from './ContextMenu';

interface QueryEditorProps {
  tab: Tab;
}

const EVENT_RUN_QUERY_CODELENS = 'larik:run-query-codelens';

// SQL keywords for basic completion
const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'IN', 'LIKE',
  'ORDER BY', 'GROUP BY', 'HAVING', 'JOIN', 'LEFT JOIN', 'RIGHT JOIN',
  'INNER JOIN', 'OUTER JOIN', 'CROSS JOIN', 'ON', 'AS', 'DISTINCT', 'TOP',
  'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET', 'DELETE',
  'CREATE', 'ALTER', 'DROP', 'TABLE', 'INDEX', 'VIEW', 'PROCEDURE', 'FUNCTION',
  'NULL', 'IS NULL', 'IS NOT NULL', 'BETWEEN', 'EXISTS',
  'UNION', 'UNION ALL', 'EXCEPT', 'INTERSECT',
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'CASE', 'WHEN', 'THEN', 'ELSE', 'END',
  'CAST', 'CONVERT', 'COALESCE', 'NULLIF', 'IIF', 'ISNULL',
  'BEGIN', 'COMMIT', 'ROLLBACK', 'TRANSACTION',
  'DECLARE', 'EXEC', 'EXECUTE', 'RETURN', 'PRINT',
  'WITH', 'CTE', 'OVER', 'PARTITION BY', 'ROW_NUMBER', 'RANK', 'DENSE_RANK',
  'OFFSET', 'FETCH', 'NEXT', 'ROWS', 'ONLY',
  'ASC', 'DESC', 'NULLS', 'FIRST', 'LAST',
  'CROSS APPLY', 'OUTER APPLY', 'PIVOT', 'UNPIVOT',
  'MERGE', 'MATCHED', 'OUTPUT', 'INSERTED', 'DELETED',
  'USE',
];

// SQL built-in functions
const SQL_FUNCTIONS = [
  // Aggregate functions
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'STDEV', 'STDEVP', 'VAR', 'VARP',
  'STRING_AGG', 'GROUPING', 'GROUPING_ID',
  // String functions
  'LEN', 'DATALENGTH', 'LEFT', 'RIGHT', 'SUBSTRING', 'CHARINDEX', 'PATINDEX',
  'REPLACE', 'STUFF', 'REPLICATE', 'REVERSE', 'LTRIM', 'RTRIM', 'TRIM',
  'UPPER', 'LOWER', 'CONCAT', 'CONCAT_WS', 'STRING_SPLIT', 'FORMAT',
  'QUOTENAME', 'SPACE', 'CHAR', 'ASCII', 'UNICODE', 'NCHAR',
  // Date/Time functions
  'GETDATE', 'GETUTCDATE', 'SYSDATETIME', 'SYSUTCDATETIME', 'SYSDATETIMEOFFSET',
  'DATEADD', 'DATEDIFF', 'DATEDIFF_BIG', 'DATEPART', 'DATENAME', 'DAY', 'MONTH', 'YEAR',
  'EOMONTH', 'DATEFROMPARTS', 'DATETIME2FROMPARTS', 'ISDATE',
  // Conversion functions
  'CAST', 'CONVERT', 'TRY_CAST', 'TRY_CONVERT', 'PARSE', 'TRY_PARSE',
  // Logical functions
  'IIF', 'CHOOSE', 'COALESCE', 'NULLIF',
  // Math functions
  'ABS', 'CEILING', 'FLOOR', 'ROUND', 'POWER', 'SQRT', 'SQUARE', 'SIGN',
  'LOG', 'LOG10', 'EXP', 'PI', 'RAND', 'SIN', 'COS', 'TAN',
  // Window functions
  'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'NTILE', 'LAG', 'LEAD',
  'FIRST_VALUE', 'LAST_VALUE', 'PERCENT_RANK', 'CUME_DIST',
  // System functions
  'NEWID', 'NEWSEQUENTIALID', 'SCOPE_IDENTITY', '@@IDENTITY', '@@ROWCOUNT',
  'ISNULL', 'ISNUMERIC', 'ISDATE', 'OBJECT_ID', 'OBJECT_NAME',
  'DB_ID', 'DB_NAME', 'SCHEMA_ID', 'SCHEMA_NAME', 'USER_ID', 'USER_NAME',
  // JSON functions
  'JSON_VALUE', 'JSON_QUERY', 'JSON_MODIFY', 'ISJSON', 'OPENJSON',
];

// Keywords where table aliases should be suggested (FROM, JOIN contexts)
const TABLE_ALIAS_KEYWORDS = ['FROM', 'JOIN', 'INNER JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'OUTER JOIN', 'CROSS JOIN', 'APPLY', 'CROSS APPLY', 'OUTER APPLY'];

// Local storage key for tracking recently used tables
const RECENT_TABLES_KEY = 'larik_recent_tables';
const MAX_RECENT_TABLES = 20;

/**
 * Track a table as recently used
 */
function trackRecentTable(schemaName: string, tableName: string): void {
  try {
    const key = `${schemaName}.${tableName}`;
    const stored = localStorage.getItem(RECENT_TABLES_KEY);
    let recentTables: string[] = stored ? JSON.parse(stored) : [];

    // Remove if already exists, then add to front
    recentTables = recentTables.filter(t => t !== key);
    recentTables.unshift(key);

    // Keep only the most recent
    recentTables = recentTables.slice(0, MAX_RECENT_TABLES);

    localStorage.setItem(RECENT_TABLES_KEY, JSON.stringify(recentTables));
  } catch {
    // Ignore localStorage errors
  }
}

/**
 * Get recently used tables
 */
function getRecentTables(): Set<string> {
  try {
    const stored = localStorage.getItem(RECENT_TABLES_KEY);
    return new Set(stored ? JSON.parse(stored) : []);
  } catch {
    return new Set();
  }
}

/**
 * Parse SQL text to extract table aliases and table references
 * Returns a map of alias -> full table name
 * Now uses AST parsing for better accuracy with CTEs, subqueries, etc.
 */
function parseTableAliases(sql: string): Map<string, { schema: string; table: string }> {
  return parseTableAliasesAST(sql);
}

/**
 * Generate an alias for a table name
 * - Single word: first letter lowercase (Application -> a)
 * - CamelCase: initials lowercase (ApplicationUser -> au)
 * - snake_case: initials lowercase (application_user -> au)
 */
function generateTableAlias(tableName: string): string {
  // Try to extract initials from CamelCase (ApplicationUser -> au)
  const camelCaseMatch = tableName.match(/[A-Z]/g);
  if (camelCaseMatch && camelCaseMatch.length > 1) {
    return camelCaseMatch.join('').toLowerCase();
  }

  // Try to extract initials from snake_case (application_user -> au)
  const snakeCaseMatch = tableName.split('_').map(part => part.charAt(0)).join('');
  if (snakeCaseMatch.length > 1) {
    return snakeCaseMatch.toLowerCase();
  }

  // Single word: just first letter
  return tableName.charAt(0).toLowerCase();
}

/**
 * Track which aliases have been used in the current query to avoid conflicts
 * Now uses AST parsing to catch all alias types (explicit, implicit, CTEs)
 */
function getUsedAliases(sql: string): Set<string> {
  return getUsedAliasesAST(sql);
}

/**
 * Generate a unique alias that doesn't conflict with existing ones
 */
function generateUniqueAlias(tableName: string, usedAliases: Set<string>): string {
  let baseAlias = generateTableAlias(tableName);
  let alias = baseAlias;
  let counter = 1;

  while (usedAliases.has(alias.toLowerCase())) {
    alias = `${baseAlias}${counter}`;
    counter++;
  }

  return alias;
}

/**
 * Extract table names referenced in the query (for tracking recently used)
 * Now uses AST parsing to recursively find tables in subqueries, CTEs, etc.
 */
function extractReferencedTables(sql: string): Array<{ schema: string; table: string }> {
  return extractReferencedTablesAST(sql);
}

/**
 * Extract database name from USE statement
 * Supports: USE DatabaseName, USE [DatabaseName], USE [database-with-special.chars]
 */
function extractUseDatabaseStatement(sql: string): string | null {
  // Match USE [database] with any characters inside brackets
  const bracketPattern = /\bUSE\s+\[([^\]]+)\]/i;
  const bracketMatch = sql.match(bracketPattern);
  if (bracketMatch) {
    return bracketMatch[1];
  }

  // Match USE database without brackets (only word characters)
  const simplePattern = /\bUSE\s+(\w+)/i;
  const simpleMatch = sql.match(simplePattern);
  return simpleMatch ? simpleMatch[1] : null;
}

/**
 * Get the context of the current cursor position
 * Now uses hybrid AST + regex approach for better accuracy with CTEs, subqueries, etc.
 */
function getCompletionContext(
  textBeforeCursor: string,
  fullText?: string
): CompletionContext {
  return getCompletionContextAST(textBeforeCursor, fullText);
}

/**
 * Build rich documentation for a column
 */
function buildColumnDocumentation(col: SchemaColumnInfo, tableName?: string): string {
  const parts: string[] = [];

  // Table info
  if (tableName) {
    parts.push(`**Table:** ${tableName}`);
  }

  // Type info
  parts.push(`**Type:** ${getDisplayDataType(col)}`);

  // Constraints
  const constraints: string[] = [];
  if (col.is_primary_key) constraints.push('Primary Key');
  if (col.is_identity) constraints.push('Identity');
  if (!col.is_nullable) constraints.push('NOT NULL');
  if (col.is_nullable) constraints.push('Nullable');

  if (constraints.length > 0) {
    parts.push(`**Constraints:** ${constraints.join(', ')}`);
  }

  // Default value
  if (col.column_default) {
    parts.push(`**Default:** ${col.column_default}`);
  }

  return parts.join('\n\n');
}

/**
 * Get display data type for a parameter
 */
function getParameterDisplayType(param: ParameterInfo): string {
  const { data_type, max_length, precision, scale } = param;

  // Types with length
  if (['varchar', 'nvarchar', 'char', 'nchar', 'varbinary', 'binary'].includes(data_type.toLowerCase())) {
    if (max_length === -1) {
      return `${data_type}(MAX)`;
    } else if (max_length !== null) {
      // For nvarchar/nchar, actual length is half of max_length
      const displayLength = data_type.toLowerCase().startsWith('n') ? max_length / 2 : max_length;
      return `${data_type}(${displayLength})`;
    }
  }

  // Types with precision and scale
  if (['decimal', 'numeric'].includes(data_type.toLowerCase())) {
    if (precision !== null && scale !== null) {
      return `${data_type}(${precision}, ${scale})`;
    }
  }

  return data_type;
}

/**
 * Build the insert text for a procedure with its parameters
 */
function buildProcedureInsertText(
  procName: string,
  schemaName: string,
  parameters: ParameterInfo[]
): string {
  const fullName = schemaName === 'dbo' ? procName : `[${schemaName}].[${procName}]`;

  if (parameters.length === 0) {
    return fullName;
  }

  // Build parameter list with named parameters
  const paramList = parameters
    .filter(p => p.parameter_mode !== 'OUT') // Only include input parameters by default
    .map((param, index) => {
      // Use tab stops for snippet navigation: ${n:default}
      return `${param.name} = \${${index + 1}:NULL}`;
    })
    .join(', ');

  return paramList ? `${fullName} ${paramList}` : fullName;
}

/**
 * Build documentation for a stored procedure showing its parameters
 */
function buildProcedureDocumentation(params: ParameterInfo[], returnType: string | null): string {
  const parts: string[] = [];

  if (params.length > 0) {
    parts.push('**Parameters:**');
    params.forEach(param => {
      const mode = param.parameter_mode !== 'IN' ? ` (${param.parameter_mode})` : '';
      parts.push(`- \`${param.name}\` ${getParameterDisplayType(param)}${mode}`);
    });
  } else {
    parts.push('*No parameters*');
  }

  if (returnType) {
    parts.push('', `**Returns:** ${returnType}`);
  }

  return parts.join('\n');
}

function QueryEditorComp({ tab }: QueryEditorProps) {

  // Atomic selectors to prevent re-renders
  const autosaveContent = useAppStore(s => s.autosaveContent);
  const isSaving = useAppStore(s => s.isSaving);
  const spaces = useAppStore(s => s.spaces);
  const activeSpaceId = useAppStore(s => s.activeSpaceId);
  const spaceConnectionStatus = useAppStore(s => s.spaceConnectionStatus);

  const executeQuery = useAppStore(s => s.executeQuery);
  const executeQueryAppend = useAppStore(s => s.executeQueryAppend);
  const executeSilentQuery = useAppStore(s => s.executeSilentQuery);
  const cancelRunningQueries = useAppStore(s => s.cancelRunningQueries);
  const clearQueryResult = useAppStore(s => s.clearQueryResult);
  const closeResult = useAppStore(s => s.closeResult);

  const setActiveResultIndex = useAppStore(s => s.setActiveResultIndex);

  const connectToSpace = useAppStore(s => s.connectToSpace);
  const schemaInfo = useAppStore(s => s.schemaInfo);
  const loadSchema = useAppStore(s => s.loadSchema);

  const setResultCustomName = useAppStore(s => s.setResultCustomName);
  const getResultCustomName = useAppStore(s => s.getResultCustomName);

  const getEnabledSnippets = useAppStore(s => s.getEnabledSnippets);
  const loadSnippets = useAppStore(s => s.loadSnippets);
  const snippets = useAppStore(s => s.snippets);

  const spaceDatabases = useAppStore(s => s.spaceDatabases);
  const loadSpaceDatabases = useAppStore(s => s.loadSpaceDatabases);
  const updateTabDatabase = useAppStore(s => s.updateTabDatabase);

  const validationEnabled = useAppStore(s => s.validationEnabled);
  const validationShowWarnings = useAppStore(s => s.validationShowWarnings);
  const validationShowInfo = useAppStore(s => s.validationShowInfo);
  const toggleValidation = useAppStore(s => s.toggleValidation);

  const enableStickyNotes = useAppStore(s => s.enableStickyNotes);

  // Get theme for Monaco editor
  const theme = useAppStore((state) => state.theme);

  // Compute effective theme (resolve 'system' to actual theme)
  const effectiveTheme = theme === 'system'
    ? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
    : theme;
  const monacoTheme = effectiveTheme === 'dark' ? 'vs-dark' : 'vs';

  // Get per-tab query results and executing state - using selectors to ensure reactivity
  const queryResults = useAppStore(s => s.tabQueryResults[tab.id] ?? null);
  const activeResultIndex = useAppStore(s => s.activeResultIndex[tab.id] ?? 0);
  const isExecuting = useAppStore(s => s.tabExecuting[tab.id] ?? false);

  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<typeof import('monaco-editor') | null>(null);
  const completionProviderRef = useRef<IDisposable | null>(null);
  const snippetProviderRef = useRef<IDisposable | null>(null);
  const codeLensProviderRef = useRef<IDisposable | null>(null);
  const saveTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastTabIdRef = useRef<string>(tab.id);
  const executeQueryRef = useRef<(() => Promise<void>) | null>(null);
  const executeQueryAppendRef = useRef<(() => Promise<void>) | null>(null);
  const [resultPanelHeight, setResultPanelHeight] = useState(450);
  const [isResizingResults, setIsResizingResults] = useState(false);
  const [hasSelection, setHasSelection] = useState(false);
  const [editingResultIndex, setEditingResultIndex] = useState<number | null>(null);
  const [editingResultName, setEditingResultName] = useState('');
  const [editorReady, setEditorReady] = useState(false);
  const [lastExecutedQuery, setLastExecutedQuery] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{ position: { x: number; y: number } } | null>(null);

  // Sticky Notes Integration
  const { StickyNotesRenderer } = useStickyNotes({
    editor: editorRef.current,
    model: editorRef.current?.getModel() || null,
    enabled: enableStickyNotes,
    onContentChange: (newContent) => {
      if (editorRef.current) {
        const fullRange = editorRef.current.getModel()?.getFullModelRange();
        if (fullRange) {
          editorRef.current.executeEdits('sticky-note', [{
            range: fullRange,
            text: newContent,
            forceMoveMarkers: true
          }]);
        }
      }
    }
  });

  // Register CodeLens and Command
  useEffect(() => {
    if (!editorReady || !monacoRef.current || !editorRef.current) return;

    const monaco = monacoRef.current;
    const editor = editorRef.current;

    // Register global command (idempotent-ish)
    try {
      // Check if command exists or just try-catch
      // Monaco doesn't have hasCommand?
      // We'll use a unique ID per editor to be safe? 
      // No, CodeLens string ID must be known.
      // Let's try to register. If it throws, it exists.
      monaco.editor.registerCommand('larik.runQuery', (_, args: { tabId: string, sql: string }) => {
        window.dispatchEvent(new CustomEvent(EVENT_RUN_QUERY_CODELENS, { detail: args }));
      });

      if (enableStickyNotes) {
        monaco.editor.registerCommand('larik.addStickyNote', (_, args: { line: number }) => {
          window.dispatchEvent(new CustomEvent(EVENT_ADD_STICKY_NOTE, { detail: { line: args.line } }));
        });
      }
    } catch (e) {
      // Command likely already registered
    }

    // Register CodeLens Provider
    if (codeLensProviderRef.current) {
      codeLensProviderRef.current.dispose();
    }

    codeLensProviderRef.current = monaco.languages.registerCodeLensProvider('sql', {
      onDidChange: (cb) => {
        const disposable = editor.onDidChangeModelContent(() => cb(editor.getModel() as any));
        return disposable;
      },
      provideCodeLenses: (model) => {
        // Only provide for this editor's model
        if (model !== editor.getModel()) return { lenses: [], dispose: () => { } };

        const sql = model.getValue();
        const statements = extractAllStatements(sql);
        const notes = extractNotes(sql);
        const noteLineNumbers = new Set(notes.map(n => n.lineNumber));

        const lenses = statements
          .map(stmt => {
            const range = {
              startLineNumber: stmt.startLine,
              startColumn: 1,
              endLineNumber: stmt.startLine,
              endColumn: 1
            };

            const resultLenses: languages.CodeLens[] = [
              {
                range,
                command: {
                  id: 'larik.runQuery',
                  title: 'Run Query',
                  arguments: [{ tabId: tab.id, sql: stmt.statement }]
                }
              }
            ];

            // Conditionally add "Add Note"
            let hasNote = false;
            let currentLine = stmt.startLine - 1;
            while (currentLine >= 1) {
              if (noteLineNumbers.has(currentLine)) {
                hasNote = true;
                break;
              }
              const lineContent = model.getLineContent(currentLine).trim();
              if (lineContent !== '') break;
              currentLine--;
            }

            if (enableStickyNotes && !hasNote) {
              resultLenses.push({
                range,
                command: {
                  id: 'larik.addStickyNote',
                  title: 'Add Note',
                  arguments: [{ line: stmt.startLine }]
                }
              });
            }

            return resultLenses;
          })
          .flat();

        return {
          lenses,
          dispose: () => { }
        };
      }
    });

    return () => {
      if (codeLensProviderRef.current) {
        codeLensProviderRef.current.dispose();
      }
    };
  }, [editorReady, tab.id, enableStickyNotes]); // Add enableStickyNotes dependency



  // Custom Copy Handler
  // Custom Copy Handler
  const handleCopy = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor) return;

    const selection = editor.getSelection();
    if (!selection || selection.isEmpty()) return;

    const model = editor.getModel();
    if (!model) return;

    const text = model.getValueInRange(selection);

    try {
      await navigator.clipboard.writeText(text);
      editor.focus();
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  }, []);

  // Custom Cut Handler
  // Custom Cut Handler
  const handleCut = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor) return;

    const selection = editor.getSelection();
    if (!selection || selection.isEmpty()) return;

    const model = editor.getModel();
    if (!model) return;

    const text = model.getValueInRange(selection);

    try {
      // Copy to clipboard
      await navigator.clipboard.writeText(text);

      // Delete selection
      editor.executeEdits('cut', [{
        range: selection,
        text: '',
        forceMoveMarkers: true
      }]);

      editor.focus();
    } catch (err) {
      console.error('Failed to cut:', err);
    }
  }, []);

  // Custom Paste Handler
  const handlePaste = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor) return;

    try {
      // Use Clipboard API for reliable paste
      const text = await navigator.clipboard.readText();
      if (!text) return;

      const selection = editor.getSelection();
      if (selection) {
        editor.executeEdits('paste', [{
          range: selection,
          text: text,
          forceMoveMarkers: true
        }]);
      }
    } catch (err) {
      console.error('Failed to paste:', err);
    }
  }, []);



  // Get space color for theming
  const activeSpace = spaces.find(s => s.id === activeSpaceId);
  const spaceColor = activeSpace?.color || '#6366f1';

  // Check connection status (1:1 model)
  const hasConnection = activeSpace ? spaceHasConnection(activeSpace) : false;
  const isConnected = spaceConnectionStatus?.is_connected ?? false;

  // Handle Run Query from CodeLens
  useEffect(() => {
    const handleRunQuery = (e: any) => {
      const { tabId, sql } = e.detail || {};
      if (tabId === tab.id && sql && hasConnection) {
        // Also need to track recent tables for this specific statement
        const referencedTables = extractReferencedTables(sql);
        referencedTables.forEach(t => trackRecentTable(t.schema, t.table));
        setLastExecutedQuery(sql);

        // Use full content for context, but execute specific SQL
        const model = editorRef.current?.getModel();
        const fullContent = model ? model.getValue() : sql;

        executeQuery(tab.id, fullContent, sql);
      } else if (tabId === tab.id && !hasConnection) {
        alert('Please configure a database connection for this space');
      }
    };

    window.addEventListener(EVENT_RUN_QUERY_CODELENS, handleRunQuery);
    return () => window.removeEventListener(EVENT_RUN_QUERY_CODELENS, handleRunQuery);
  }, [tab.id, hasConnection, executeQuery]);


  // Context Menu Items
  const contextMenuItems: ContextMenuItem[] = [
    {
      id: 'copy',
      label: 'Copy',
      shortcut: 'Ctrl+C',
      action: handleCopy,
      disabled: !hasSelection,
    },
    {
      id: 'cut',
      label: 'Cut',
      shortcut: 'Ctrl+X',
      action: handleCut,
      disabled: !hasSelection,
    },
    {
      id: 'paste',
      label: 'Paste',
      shortcut: 'Ctrl+V',
      action: handlePaste,
    },
    {
      id: 'separator-1',
      label: '',
      action: () => { },
      separator: true,
    },
    {
      id: 'run-selection',
      label: 'Run Selection',
      shortcut: 'Ctrl+Shift+Enter',
      action: () => executeQueryRef.current?.(),
      disabled: !hasSelection || !hasConnection,
    },
    {
      id: 'run',
      label: 'Run Query',
      shortcut: 'Ctrl+Enter',
      action: () => executeQueryRef.current?.(),
      disabled: !hasConnection,
    },
    {
      id: 'separator-2',
      label: '',
      action: () => { },
      separator: true,
    },
    {
      id: 'format',
      label: 'Format Document',
      shortcut: 'Ctrl+Alt+F',
      action: () => formatQueryRef.current?.(),
    },
  ];

  // Update editor content when switching tabs (since we use defaultValue)
  useEffect(() => {
    if (editorRef.current && tab.id !== lastTabIdRef.current) {
      const currentValue = editorRef.current.getValue();
      const newValue = tab.content || '';
      // Only update if content actually changed (different tab)
      if (currentValue !== newValue) {
        editorRef.current.setValue(newValue);
      }
      lastTabIdRef.current = tab.id;
    }
  }, [tab.id, tab.content]);

  // Focus editor when tab becomes active
  useEffect(() => {
    if (editorRef.current && editorReady) {
      editorRef.current.focus();
    }
  }, [tab.id, editorReady]);

  // Load schema when connected and tab database changes
  useEffect(() => {
    if (isConnected && tab.database) {
      loadSchema(tab.database);
    }
  }, [isConnected, tab.database, loadSchema]);

  // Load snippets on mount (if not already loaded)
  useEffect(() => {
    console.log('[QueryEditor] Checking snippets, current count:', snippets.length);
    if (snippets.length === 0) {
      console.log('[QueryEditor] No snippets, calling loadSnippets()');
      loadSnippets();
    }
  }, [snippets.length, loadSnippets]);

  // Load databases when connected
  useEffect(() => {
    if (isConnected) {
      loadSpaceDatabases();
    }
  }, [isConnected, loadSpaceDatabases]);

  // Create completion suggestions from schema
  const createSchemaCompletions = useCallback((
    monaco: typeof import('monaco-editor'),
    schemaData: SchemaInfo | null,
    context: ReturnType<typeof getCompletionContext>,
    tableAliases: Map<string, { schema: string; table: string }>,
    range: IRange,
    fullText: string,
    databases: string[]
  ): languages.CompletionItem[] => {
    const suggestions: languages.CompletionItem[] = [];
    const recentTables = getRecentTables();

    // Database completion (after USE)
    if (context.type === 'database') {
      databases.forEach(dbName => {
        suggestions.push({
          label: `[${dbName}]`,
          kind: monaco.languages.CompletionItemKind.Module,
          detail: 'Database',
          documentation: `Switch to database: ${dbName}`,
          insertText: `[${dbName}]`,
          sortText: `000_${dbName}`,
          range,
        });
      });

      return suggestions;
    }

    if (!schemaData) return suggestions;

    // UPDATE column completion (only columns from the table being updated)
    if (context.type === 'update_column' && context.targetTable) {
      const targetTable = schemaData.tables.find(t =>
        t.table_name.toLowerCase() === context.targetTable!.table.toLowerCase() &&
        t.schema_name.toLowerCase() === context.targetTable!.schema.toLowerCase()
      );

      if (targetTable) {
        // Check if we're in SET clause (add = ) or WHERE clause (no = )
        const isSetClause = /\bSET\b(?!.*\bWHERE\b)/i.test(fullText);

        targetTable.columns.forEach(col => {
          suggestions.push({
            label: col.name,
            kind: monaco.languages.CompletionItemKind.Field,
            detail: getDisplayDataType(col),
            documentation: { value: buildColumnDocumentation(col, targetTable.table_name), isTrusted: true },
            insertText: isSetClause ? `[${col.name}] = ` : `[${col.name}]`,
            sortText: `0_${col.ordinal_position.toString().padStart(3, '0')}`,
            range,
          });
        });
      }

      return suggestions;
    }

    // INSERT column completion (only columns from the table being inserted)
    if (context.type === 'insert_column' && context.targetTable) {
      const targetTable = schemaData.tables.find(t =>
        t.table_name.toLowerCase() === context.targetTable!.table.toLowerCase() &&
        t.schema_name.toLowerCase() === context.targetTable!.schema.toLowerCase()
      );

      if (targetTable) {
        targetTable.columns.forEach(col => {
          suggestions.push({
            label: col.name,
            kind: monaco.languages.CompletionItemKind.Field,
            detail: getDisplayDataType(col),
            documentation: { value: buildColumnDocumentation(col, targetTable.table_name), isTrusted: true },
            insertText: `[${col.name}]`,
            sortText: `0_${col.ordinal_position.toString().padStart(3, '0')}`,
            range,
          });
        });
      }

      return suggestions;
    }

    // Alias column completion (e.g., "u." should show columns from users table)
    if (context.type === 'alias_column' && context.alias) {
      const aliasInfo = tableAliases.get(context.alias);
      if (aliasInfo) {
        const table = schemaData.tables.find(t =>
          t.table_name.toLowerCase() === aliasInfo.table.toLowerCase() &&
          t.schema_name.toLowerCase() === aliasInfo.schema.toLowerCase()
        );
        if (table) {
          table.columns.forEach(col => {
            suggestions.push({
              label: col.name,
              kind: monaco.languages.CompletionItemKind.Field,
              detail: getDisplayDataType(col),
              documentation: { value: buildColumnDocumentation(col, table.table_name), isTrusted: true },
              insertText: col.name,
              sortText: `0_${col.ordinal_position.toString().padStart(3, '0')}`,
              range,
            });
          });
          return suggestions;
        }
      }

      // Also check if alias matches a table name directly (e.g., "Users." when no alias defined)
      const directTable = schemaData.tables.find(t =>
        t.table_name.toLowerCase() === context.alias
      );
      if (directTable) {
        directTable.columns.forEach(col => {
          suggestions.push({
            label: col.name,
            kind: monaco.languages.CompletionItemKind.Field,
            detail: getDisplayDataType(col),
            documentation: { value: buildColumnDocumentation(col, directTable.table_name), isTrusted: true },
            insertText: col.name,
            sortText: `0_${col.ordinal_position.toString().padStart(3, '0')}`,
            range,
          });
        });
        return suggestions;
      }

      // Check if it's a schema prefix (e.g., "dbo.")
      if (['dbo', 'sys'].includes(context.alias)) {
        schemaData.tables
          .filter(t => t.schema_name.toLowerCase() === context.alias)
          .forEach(table => {
            const isView = table.table_type === 'VIEW';
            const isRecent = recentTables.has(`${table.schema_name}.${table.table_name}`);

            suggestions.push({
              label: table.table_name,
              kind: isView ? monaco.languages.CompletionItemKind.Interface : monaco.languages.CompletionItemKind.Struct,
              detail: isView ? 'View' : 'Table',
              documentation: { value: `**Columns:**\n${table.columns.map(c => `- ${c.name} (${getDisplayDataType(c)})`).join('\n')}`, isTrusted: true },
              insertText: table.table_name,
              sortText: isRecent ? `0_${table.table_name}` : `1_${table.table_name}`,
              range,
            });
          });

        // Also add routines for this schema
        schemaData.routines
          .filter(r => r.schema_name.toLowerCase() === context.alias)
          .forEach(routine => {
            const paramCount = (routine.parameters || []).filter(p => p.parameter_mode !== 'OUT').length;
            suggestions.push({
              label: routine.routine_name,
              kind: routine.routine_type === 'FUNCTION'
                ? monaco.languages.CompletionItemKind.Function
                : monaco.languages.CompletionItemKind.Method,
              detail: `${routine.routine_type}${paramCount > 0 ? ` (${paramCount} params)` : ''}`,
              documentation: { value: buildProcedureDocumentation(routine.parameters || [], routine.return_type), isTrusted: true },
              insertText: routine.routine_name,
              sortText: `2_${routine.routine_name}`,
              range,
            });
          });

        return suggestions;
      }

      return suggestions;
    }

    // Routine completion (after EXEC/EXECUTE)
    if (context.type === 'routine') {
      // Stored procedures
      schemaData.routines
        .filter(r => r.routine_type === 'PROCEDURE')
        .forEach(proc => {
          const hasParams = proc.parameters && proc.parameters.length > 0;
          const insertText = buildProcedureInsertText(proc.routine_name, proc.schema_name, proc.parameters || []);
          const paramCount = (proc.parameters || []).filter(p => p.parameter_mode !== 'OUT').length;

          suggestions.push({
            label: proc.routine_name,
            kind: monaco.languages.CompletionItemKind.Method,
            detail: `${proc.schema_name} • Stored Procedure${paramCount > 0 ? ` (${paramCount} params)` : ''}`,
            documentation: { value: buildProcedureDocumentation(proc.parameters || [], proc.return_type), isTrusted: true },
            insertText: insertText,
            insertTextRules: hasParams ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
            sortText: `0_${proc.routine_name}`,
            range,
          });

          // Add schema-qualified version for non-dbo
          if (proc.schema_name !== 'dbo') {
            suggestions.push({
              label: `[${proc.schema_name}].[${proc.routine_name}]`,
              kind: monaco.languages.CompletionItemKind.Method,
              detail: `Stored Procedure${paramCount > 0 ? ` (${paramCount} params)` : ''}`,
              documentation: { value: buildProcedureDocumentation(proc.parameters || [], proc.return_type), isTrusted: true },
              insertText: insertText,
              insertTextRules: hasParams ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
              sortText: `1_${proc.routine_name}`,
              range,
            });
          }
        });

      return suggestions;
    }

    // Table completion
    if (context.type === 'table' || context.type === 'schema') {
      // Check if aliases should be suggested based on the keyword context
      const shouldSuggestAliases = context.lastKeyword && TABLE_ALIAS_KEYWORDS.includes(context.lastKeyword);

      // Get used aliases to avoid conflicts (only if we're suggesting aliases)
      const usedAliases = shouldSuggestAliases ? getUsedAliases(fullText) : new Set<string>();

      // Add existing aliases from the query (CTEs, subquery aliases, etc.)
      // This allows referencing aliases defined earlier in the query
      const existingAliases = parseTableAliases(fullText);
      existingAliases.forEach((tableInfo, alias) => {
        // Skip if it's a subquery alias (we don't have schema info for it)
        if (tableInfo.schema === 'subquery') {
          suggestions.push({
            label: alias,
            kind: monaco.languages.CompletionItemKind.Reference,
            detail: `Subquery alias`,
            documentation: `Reference to subquery aliased as '${alias}'`,
            insertText: alias,
            sortText: `00_${alias}`, // High priority
            range,
          });
          return;
        }

        // For CTE or table aliases, suggest them
        const isCTE = tableInfo.schema === 'cte';
        suggestions.push({
          label: alias,
          kind: isCTE ? monaco.languages.CompletionItemKind.Reference : monaco.languages.CompletionItemKind.Variable,
          detail: isCTE ? `CTE: ${tableInfo.table}` : `Alias: ${tableInfo.schema}.${tableInfo.table}`,
          documentation: isCTE
            ? `Common Table Expression (WITH clause) named '${tableInfo.table}'`
            : `Alias '${alias}' references table ${tableInfo.schema}.${tableInfo.table}`,
          insertText: alias,
          sortText: `00_${alias}`, // High priority - existing aliases at top
          range,
        });
      });

      schemaData.tables.forEach(table => {
        const fullName = `[${table.schema_name}].[${table.table_name}]`;
        const isView = table.table_type === 'VIEW';
        const isRecent = recentTables.has(`${table.schema_name}.${table.table_name}`);

        // Build column list for documentation
        const columnDocs = table.columns.slice(0, 15).map(c => {
          const markers = [];
          if (c.is_primary_key) markers.push('🔑');
          if (!c.is_nullable) markers.push('*');
          return `- ${c.name} (${getDisplayDataType(c)})${markers.length ? ' ' + markers.join('') : ''}`;
        }).join('\n');
        const hasMore = table.columns.length > 15 ? `\n- ... and ${table.columns.length - 15} more` : '';

        if (shouldSuggestAliases) {
          // Generate unique alias for this table
          const alias = generateUniqueAlias(table.table_name, usedAliases);
          // NOTE: Don't add to usedAliases here - we only track aliases already in the query,
          // not the ones we're suggesting. Each table gets its own suggestion independently.

          // Insert text with proper formatting: [schema].[Table] AS [alias]
          const insertTextWithAlias = `[${table.schema_name}].[${table.table_name}] AS [${alias}]`;

          suggestions.push({
            label: table.table_name,
            kind: isView ? monaco.languages.CompletionItemKind.Interface : monaco.languages.CompletionItemKind.Struct,
            detail: `${table.schema_name} • ${isView ? 'View' : 'Table'} → AS [${alias}]`,
            documentation: { value: `**Inserts:** \`${insertTextWithAlias}\`\n\n**Columns:**\n${columnDocs}${hasMore}\n\n🔑 = Primary Key, * = NOT NULL`, isTrusted: true },
            insertText: insertTextWithAlias,
            sortText: isRecent ? `0_${table.table_name}` : `1_${table.table_name}`,
            range,
          });

          // Also add version without alias for cases where you don't want one
          suggestions.push({
            label: `${table.table_name} (no alias)`,
            kind: isView ? monaco.languages.CompletionItemKind.Interface : monaco.languages.CompletionItemKind.Struct,
            detail: `${table.schema_name} • ${isView ? 'View' : 'Table'}`,
            documentation: { value: `**Inserts:** \`${fullName}\`\n\n**Columns:**\n${columnDocs}${hasMore}`, isTrusted: true },
            insertText: fullName,
            sortText: isRecent ? `0z_${table.table_name}` : `1z_${table.table_name}`,
            range,
          });
        } else {
          // For UPDATE, INSERT INTO, etc. - only suggest table name without alias
          suggestions.push({
            label: table.table_name,
            kind: isView ? monaco.languages.CompletionItemKind.Interface : monaco.languages.CompletionItemKind.Struct,
            detail: `${table.schema_name} • ${isView ? 'View' : 'Table'}`,
            documentation: { value: `**Inserts:** \`${fullName}\`\n\n**Columns:**\n${columnDocs}${hasMore}\n\n🔑 = Primary Key, * = NOT NULL`, isTrusted: true },
            insertText: fullName,
            sortText: isRecent ? `0_${table.table_name}` : `1_${table.table_name}`,
            range,
          });
        }
      });

      // Add functions that can be used in FROM (table-valued functions)
      schemaData.routines
        .filter(r => r.routine_type === 'FUNCTION')
        .forEach(func => {
          const fullName = func.schema_name === 'dbo'
            ? func.routine_name
            : `[${func.schema_name}].[${func.routine_name}]`;

          suggestions.push({
            label: func.routine_name + '()',
            kind: monaco.languages.CompletionItemKind.Function,
            detail: `${func.schema_name} • Function`,
            documentation: func.return_type ? `Returns: ${func.return_type}` : 'Table-valued Function',
            insertText: fullName + '()',
            sortText: `3_${func.routine_name}`,
            range,
          });
        });
    }

    // Column completion
    if (context.type === 'column' || context.type === 'function') {
      // Get all columns from tables that have been mentioned in the query
      const mentionedTables = new Set<string>();
      tableAliases.forEach(info => mentionedTables.add(`${info.schema}.${info.table}`.toLowerCase()));

      // Also add tables with aliases for prefix suggestions (e.g., suggest "[a].ColumnName")
      const aliasEntries = Array.from(tableAliases.entries());

      schemaData.tables.forEach(table => {
        const tableKey = `${table.schema_name}.${table.table_name}`.toLowerCase();
        const isRelevant = mentionedTables.has(tableKey);

        // Find if this table has an alias
        const aliasEntry = aliasEntries.find(([_, info]) =>
          info.schema.toLowerCase() === table.schema_name.toLowerCase() &&
          info.table.toLowerCase() === table.table_name.toLowerCase()
        );
        const alias = aliasEntry ? aliasEntry[0] : null;

        if (isRelevant || mentionedTables.size === 0) {
          table.columns.forEach(col => {
            // If table has an alias, show [alias].[column] format with filterText for searching
            if (alias && isRelevant) {
              suggestions.push({
                label: `[${alias}].[${col.name}]`,
                kind: monaco.languages.CompletionItemKind.Field,
                detail: `${getDisplayDataType(col)} • ${table.table_name}`,
                documentation: { value: buildColumnDocumentation(col, table.table_name), isTrusted: true },
                insertText: `[${alias}].[${col.name}]`,
                // Allow filtering by column name OR alias - so typing "id" finds "[a].[Id]"
                filterText: `${col.name} ${alias}.${col.name} [${alias}].[${col.name}]`,
                sortText: `0_${alias}_${col.name}`,
                range,
              });
            }

            // Add the column name directly (lower priority when aliases exist for relevant tables)
            if (!alias || !isRelevant) {
              suggestions.push({
                label: col.name,
                kind: monaco.languages.CompletionItemKind.Field,
                detail: `${getDisplayDataType(col)} • ${table.table_name}`,
                documentation: { value: buildColumnDocumentation(col, table.table_name), isTrusted: true },
                insertText: col.name,
                sortText: isRelevant ? `1_${col.name}` : `5_${col.name}`,
                range,
              });
            }
          });
        }
      });

      // Add SQL built-in functions in column context
      SQL_FUNCTIONS.forEach(func => {
        suggestions.push({
          label: func + '()',
          kind: monaco.languages.CompletionItemKind.Function,
          detail: 'Built-in Function',
          insertText: func + '($0)',
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          sortText: `7_${func}`,
          range,
        });
      });
    }

    return suggestions;
  }, []);

  /**
   * Detect and extract the SQL statement block at the cursor position
   * A block is delimited by ; GO or end of text
   */
  const findCurrentSqlBlock = useCallback((fullText: string, cursorLine: number, cursorColumn: number): { text: string; startLine: number; startColumn: number; endLine: number; endColumn: number } | null => {
    const lines = fullText.split('\n');

    // Find the start of the current block (scanning backwards from cursor)
    let blockStartLine = 0;
    let blockStartColumn = 0;
    let found = false;

    for (let i = cursorLine; i >= 0; i--) {
      const line = lines[i];

      // Look for statement delimiters: ; or GO (case-insensitive, word boundary)
      const delimiterMatch = line.match(/[;]|(\bGO\b)/i);

      if (delimiterMatch) {
        const delimiterEnd = delimiterMatch.index! + delimiterMatch[0].length;

        // If on current line, check if delimiter is before cursor
        if (i === cursorLine) {
          // If delimiter ends after or at cursor, skip it (cursor is within or after this delimiter)
          if (delimiterEnd >= cursorColumn) {
            continue;
          }
        }

        // Delimiter is before cursor - use it as block start
        blockStartLine = i;
        blockStartColumn = delimiterEnd;
        found = true;
        break;
      }
    }

    // If no delimiter found going backwards, start from beginning of file
    if (!found) {
      blockStartLine = 0;
      blockStartColumn = 0;
    }

    // Find the end of the current block (scanning forwards from cursor)
    let blockEndLine = cursorLine;
    let blockEndColumn = lines[cursorLine].length;

    for (let i = cursorLine; i < lines.length; i++) {
      const line = lines[i];

      // Look for statement delimiters: ; or GO (case-insensitive, word boundary)
      const delimiterMatch = line.match(/[;]|(\bGO\b)/i);

      if (delimiterMatch) {
        const delimiterStart = delimiterMatch.index!;

        // If on current line, only use delimiters after the cursor
        if (i === cursorLine) {
          // If delimiter starts before cursor, skip it and continue to next line
          if (delimiterStart < cursorColumn) {
            continue;
          }
        }

        blockEndLine = i;
        blockEndColumn = delimiterStart;
        break;
      } else if (i === lines.length - 1) {
        // End of document
        blockEndLine = i;
        blockEndColumn = line.length;
      }
    }

    // Extract the block text
    let blockText = '';

    if (blockStartLine === blockEndLine) {
      // Single line block
      blockText = lines[blockStartLine].substring(blockStartColumn, blockEndColumn);
    } else {
      // Multi-line block
      const parts: string[] = [];

      // First line
      parts.push(lines[blockStartLine].substring(blockStartColumn));

      // Middle lines
      for (let i = blockStartLine + 1; i < blockEndLine; i++) {
        parts.push(lines[i]);
      }

      // Last line
      parts.push(lines[blockEndLine].substring(0, blockEndColumn));

      blockText = parts.join('\n');
    }

    // Return null if block is empty or whitespace only
    if (!blockText.trim()) {
      return null;
    }

    return {
      text: blockText,
      startLine: blockStartLine,
      startColumn: blockStartColumn,
      endLine: blockEndLine,
      endColumn: blockEndColumn,
    };
  }, []);

  // Execute query handler
  const handleExecuteQuery = useCallback(async () => {
    if (!hasConnection) {
      alert('Please configure a database connection for this space');
      return;
    }

    if (!isConnected) {
      // Try to connect first
      const connected = await connectToSpace();
      if (!connected) {
        alert('Failed to connect to database. Please check connection settings.');
        return;
      }
    }

    const editor = editorRef.current;
    if (!editor) return;

    const model = editor.getModel();
    if (!model) return;

    // Get full query text
    const fullQuery = model.getValue();
    if (!fullQuery.trim()) {
      return;
    }

    // Check if there's a text selection
    const selection = editor.getSelection();
    let selectedText: string | null = null;
    let queryToTrack = fullQuery;

    if (selection && !selection.isEmpty()) {
      selectedText = model.getValueInRange(selection);
      // Only use selection if it's not empty/whitespace
      if (selectedText.trim()) {
        queryToTrack = selectedText;
      } else {
        selectedText = null;
      }
    } else {
      // No selection: use smart statement detection
      const cursorPosition = editor.getPosition();
      if (cursorPosition) {
        // Try smart extraction with node-sql-parser
        let statementLocation = extractStatementAtCursor(
          fullQuery,
          cursorPosition.lineNumber,
          cursorPosition.column
        );

        // Fallback to regex-based detection if parsing failed
        if (!statementLocation) {
          statementLocation = findCurrentSqlBlockFallback(
            fullQuery,
            cursorPosition.lineNumber,
            cursorPosition.column
          );
        }

        // Also fallback to old method if new methods fail
        if (!statementLocation) {
          const block = findCurrentSqlBlock(fullQuery, cursorPosition.lineNumber - 1, cursorPosition.column - 1);
          if (block) {
            statementLocation = {
              statement: block.text,
              startLine: block.startLine + 1,
              startColumn: block.startColumn + 1,
              endLine: block.endLine + 1,
              endColumn: block.endColumn + 1,
              statementIndex: 0,
            };
          }
        }

        if (statementLocation) {
          // Auto-select the detected statement for visual feedback
          editor.setSelection({
            startLineNumber: statementLocation.startLine,
            startColumn: statementLocation.startColumn,
            endLineNumber: statementLocation.endLine,
            endColumn: statementLocation.endColumn,
          });

          // Use the detected statement
          selectedText = statementLocation.statement;
          queryToTrack = statementLocation.statement;
        }
      }
    }

    // Check for USE [database] statement and auto-switch database
    const usedDatabase = extractUseDatabaseStatement(queryToTrack);
    if (usedDatabase && spaceDatabases.some(db => db.name === usedDatabase)) {
      // Update tab's database before executing
      await updateTabDatabase(tab.id, usedDatabase);
      // Reload schema for the new database
      await loadSchema(usedDatabase);
    }

    // Track recently used tables for prioritization in autocomplete
    const referencedTables = extractReferencedTables(queryToTrack);
    referencedTables.forEach(t => trackRecentTable(t.schema, t.table));

    // Store the executed query for ResultsGrid table name extraction
    setLastExecutedQuery(selectedText || fullQuery);

    // Execute with selected text (if any) or full query
    await executeQuery(tab.id, fullQuery, selectedText);
  }, [hasConnection, isConnected, connectToSpace, executeQuery, tab.id, findCurrentSqlBlock, spaceDatabases, updateTabDatabase, loadSchema]);

  // Execute query and append to existing results (Ctrl+\)
  const handleExecuteQueryAppend = useCallback(async () => {
    if (!hasConnection) {
      alert('Please configure a database connection for this space');
      return;
    }

    if (!isConnected) {
      // Try to connect first
      const connected = await connectToSpace();
      if (!connected) {
        alert('Failed to connect to database. Please check connection settings.');
        return;
      }
    }

    const editor = editorRef.current;
    if (!editor) return;

    const model = editor.getModel();
    if (!model) return;

    // Get full query text
    const fullQuery = model.getValue();
    if (!fullQuery.trim()) {
      return;
    }

    // Check if there's a text selection
    const selection = editor.getSelection();
    let selectedText: string | null = null;
    let queryToTrack = fullQuery;

    if (selection && !selection.isEmpty()) {
      selectedText = model.getValueInRange(selection);
      // Only use selection if it's not empty/whitespace
      if (selectedText.trim()) {
        queryToTrack = selectedText;
      } else {
        selectedText = null;
      }
    } else {
      // No selection: use smart statement detection
      const cursorPosition = editor.getPosition();
      if (cursorPosition) {
        // Try smart extraction with node-sql-parser
        let statementLocation = extractStatementAtCursor(
          fullQuery,
          cursorPosition.lineNumber,
          cursorPosition.column
        );

        // Fallback to regex-based detection if parsing failed
        if (!statementLocation) {
          statementLocation = findCurrentSqlBlockFallback(
            fullQuery,
            cursorPosition.lineNumber,
            cursorPosition.column
          );
        }

        // Also fallback to old method if new methods fail
        if (!statementLocation) {
          const block = findCurrentSqlBlock(fullQuery, cursorPosition.lineNumber - 1, cursorPosition.column - 1);
          if (block) {
            statementLocation = {
              statement: block.text,
              startLine: block.startLine + 1,
              startColumn: block.startColumn + 1,
              endLine: block.endLine + 1,
              endColumn: block.endColumn + 1,
              statementIndex: 0,
            };
          }
        }

        if (statementLocation) {
          // Auto-select the detected statement for visual feedback
          editor.setSelection({
            startLineNumber: statementLocation.startLine,
            startColumn: statementLocation.startColumn,
            endLineNumber: statementLocation.endLine,
            endColumn: statementLocation.endColumn,
          });

          // Use the detected statement
          selectedText = statementLocation.statement;
          queryToTrack = statementLocation.statement;
        }
      }
    }

    // Check for USE [database] statement and auto-switch database
    const usedDatabase = extractUseDatabaseStatement(queryToTrack);
    if (usedDatabase && spaceDatabases.some(db => db.name === usedDatabase)) {
      // Update tab's database before executing
      await updateTabDatabase(tab.id, usedDatabase);
      // Reload schema for the new database
      await loadSchema(usedDatabase);
    }

    // Track recently used tables for prioritization in autocomplete
    const referencedTables = extractReferencedTables(queryToTrack);
    referencedTables.forEach(t => trackRecentTable(t.schema, t.table));

    // Store the executed query for ResultsGrid table name extraction
    setLastExecutedQuery(selectedText || fullQuery);

    // Execute with selected text (if any) or full query - APPEND to existing results
    await executeQueryAppend(tab.id, fullQuery, selectedText);
  }, [hasConnection, isConnected, connectToSpace, executeQueryAppend, tab.id, findCurrentSqlBlock, spaceDatabases, updateTabDatabase, loadSchema]);

  // Execute UPDATE query for inline grid editing (silent - doesn't update results grid)
  const handleExecuteUpdateQuery = useCallback(async (updateQuery: string): Promise<boolean> => {
    if (!isConnected || !activeSpaceId) {
      console.error('Cannot execute update: not connected');
      return false;
    }

    try {
      const result = await executeSilentQuery(tab.id, updateQuery);
      if (!result.success) {
        console.error('Update failed:', result.error);
        return false;
      }
      return true;
    } catch (error) {
      console.error('Update execution failed:', error);
      return false;
    }
  }, [isConnected, activeSpaceId, executeSilentQuery, tab.id]);

  // Handle drag end for reordering results tabs
  const handleDragEnd = (result: DropResult) => {
    if (!result.destination) return;

    const sourceIndex = result.source.index;
    const destinationIndex = result.destination.index;

    if (sourceIndex === destinationIndex) return;

    reorderQueryResults(tab.id, sourceIndex, destinationIndex);
  };

  // Format SQL handler
  const handleFormatSql = useCallback(() => {
    const editor = editorRef.current;
    if (!editor) return;

    const model = editor.getModel();
    if (!model) return;

    const fullText = model.getValue();
    if (!fullText.trim()) return;

    // Check if there's a text selection
    const selection = editor.getSelection();

    if (selection && !selection.isEmpty()) {
      // Format only selected text
      const selectedText = model.getValueInRange(selection);
      const formatted = formatSqlWithIndentation(selectedText);

      // Replace selected text with formatted version
      editor.executeEdits('', [
        {
          range: selection,
          text: formatted,
        }
      ]);
    } else {
      // Format entire document
      const formatted = formatSqlWithIndentation(fullText);
      const fullRange = model.getFullModelRange();

      editor.executeEdits('', [
        {
          range: fullRange,
          text: formatted,
        }
      ]);
    }
  }, []);

  // Debounced autosave
  const handleChange: OnChange = useCallback((value) => {
    if (value === undefined || value === tab.content) return;

    // Clear existing timeout
    if (saveTimeoutRef.current) {
      clearTimeout(saveTimeoutRef.current);
    }

    // Debounce save by 400ms
    saveTimeoutRef.current = setTimeout(() => {
      autosaveContent(tab.id, value);
    }, 400);
  }, [tab.id, tab.content, autosaveContent]);

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
    };
  }, []);

  // Update completion provider when schema changes
  useEffect(() => {
    if (!monacoRef.current || !editorRef.current) return;

    const monaco = monacoRef.current;

    // Dispose previous provider
    if (completionProviderRef.current) {
      completionProviderRef.current.dispose();
    }

    // Register new completion provider with updated schema
    completionProviderRef.current = monaco.languages.registerCompletionItemProvider('sql', {
      triggerCharacters: ['.', ' ', '['],
      provideCompletionItems: (model, position) => {
        const word = model.getWordUntilPosition(position);
        let completionRange: IRange = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        };

        // Get text before cursor for context analysis
        const textBeforeCursor = model.getValueInRange({
          startLineNumber: 1,
          startColumn: 1,
          endLineNumber: position.lineNumber,
          endColumn: position.column,
        });

        const fullText = model.getValue();
        const context = getCompletionContext(textBeforeCursor, fullText);

        if (textBeforeCursor.endsWith('[')) {
          completionRange = {
            ...completionRange,
            startColumn: position.column - 1
          };
        }

        // Get current SQL block to parse aliases only from the current statement
        const currentBlock = findCurrentSqlBlock(fullText, position.lineNumber - 1, position.column - 1);
        const currentBlockText = currentBlock ? currentBlock.text : fullText;

        // Parse aliases from currentBlockText (current statement only)
        // This ensures aliases from other statements don't pollute suggestions
        const tableAliases = parseTableAliases(currentBlockText);

        const suggestions: languages.CompletionItem[] = [];

        // Add SQL keywords (lower priority)
        if (context.type === 'keyword' || context.type === 'column') {
          SQL_KEYWORDS.forEach(keyword => {
            suggestions.push({
              label: keyword,
              kind: monaco.languages.CompletionItemKind.Keyword,
              insertText: keyword,
              sortText: `9_${keyword}`,
              range: completionRange,
            });
          });
        }

        // Add schema-based completions (use current block text for context-aware suggestions)
        const schemaCompletions = createSchemaCompletions(monaco, schemaInfo, context, tableAliases, completionRange, currentBlockText, spaceDatabases.map(db => db.name));
        suggestions.push(...schemaCompletions);

        return { suggestions };
      },
    });

    return () => {
      if (completionProviderRef.current) {
        completionProviderRef.current.dispose();
      }
    };
  }, [schemaInfo, createSchemaCompletions, spaceDatabases]);

  // Update snippet completion provider when snippets change or editor becomes ready
  useEffect(() => {
    if (!monacoRef.current || !editorRef.current || !editorReady) {
      console.log('[QueryEditor] Snippet provider effect: editor not ready yet');
      return;
    }

    const monaco = monacoRef.current;
    const enabledSnippets = getEnabledSnippets();

    console.log('[QueryEditor] Snippet provider effect running, enabled snippets:', enabledSnippets.length);

    // Dispose previous snippet provider
    if (snippetProviderRef.current) {
      snippetProviderRef.current.dispose();
    }

    if (enabledSnippets.length === 0) {
      console.log('[QueryEditor] No enabled snippets, skipping provider registration');
      return;
    }

    console.log('[QueryEditor] Registering snippet provider with', enabledSnippets.length, 'snippets');

    // Register snippet completion provider
    snippetProviderRef.current = monaco.languages.registerCompletionItemProvider('sql', {
      // Don't use triggerCharacters - snippets are triggered by typing the trigger text
      provideCompletionItems: (model, position) => {
        const word = model.getWordUntilPosition(position);
        const range: IRange = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        };

        // Get the text of the current word being typed
        const currentWord = word.word.toLowerCase();

        // Create snippet suggestions
        const suggestions: languages.CompletionItem[] = enabledSnippets
          .filter(snippet =>
            // Match if the trigger starts with the current word
            snippet.trigger.toLowerCase().startsWith(currentWord) ||
            // Or if the name contains the current word
            snippet.name.toLowerCase().includes(currentWord)
          )
          .map(snippet => {
            // Convert ${cursor} placeholder to Monaco snippet format $0
            // Also convert ${1:placeholder} style to Monaco format
            let insertText = snippet.content
              .replace(/\$\{cursor\}/gi, '$0')
              .replace(/\$\{(\d+):([^}]+)\}/g, '${$1:$2}');

            return {
              label: snippet.trigger,
              kind: monaco.languages.CompletionItemKind.Snippet,
              detail: snippet.name,
              documentation: {
                value: `**${snippet.name}**${snippet.description ? `\n\n${snippet.description}` : ''}\n\n\`\`\`sql\n${snippet.content}\n\`\`\`${snippet.category ? `\n\n*Category: ${snippet.category}*` : ''}`,
                isTrusted: true,
              },
              insertText: insertText,
              insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
              // High priority for snippets
              sortText: `00_${snippet.trigger}`,
              range,
            };
          });

        return { suggestions };
      },
    });

    return () => {
      if (snippetProviderRef.current) {
        snippetProviderRef.current.dispose();
      }
    };
  }, [snippets, getEnabledSnippets, editorReady]);

  // Keep the ref updated with the latest execute handler
  useEffect(() => {
    executeQueryRef.current = handleExecuteQuery;
  }, [handleExecuteQuery]);

  // Keep the ref updated with the latest execute append handler
  useEffect(() => {
    executeQueryAppendRef.current = handleExecuteQueryAppend;
  }, [handleExecuteQueryAppend]);

  // Keep the ref updated with the latest format handler
  const formatQueryRef = useRef(handleFormatSql);
  useEffect(() => {
    formatQueryRef.current = handleFormatSql;
  }, [handleFormatSql]);

  // SQL Validation - real-time syntax and schema validation
  useSqlValidation({
    editor: editorRef.current,
    monaco: monacoRef.current,
    schemaInfo: isConnected ? schemaInfo : null, // Only validate against schema when connected
    enabled: validationEnabled,
    showWarnings: validationShowWarnings,
    showInfo: validationShowInfo,
    debounceMs: 500,
  });

  const handleEditorMount: OnMount = useCallback((editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    setEditorReady(true);
    console.log('[QueryEditor] Editor mounted');

    // Attach custom context menu listener
    editor.onContextMenu((e) => {
      e.event.preventDefault();
      setContextMenu({
        position: { x: e.event.posx, y: e.event.posy }
      });
    });

    // Add Ctrl+Enter to execute - use ref to always get the current handler
    editor.addAction({
      id: 'execute-query',
      label: 'Execute Query',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
      contextMenuGroupId: 'execution',
      contextMenuOrder: 1,
      run: () => {
        executeQueryRef.current?.();
      },
    });

    // Add F5 to execute query (run all or selection)
    editor.addAction({
      id: 'execute-query-f5',
      label: 'Execute Query',
      keybindings: [monaco.KeyCode.F5],
      contextMenuGroupId: 'execution',
      contextMenuOrder: 0,
      run: () => {
        executeQueryRef.current?.();
      },
    });

    // Add Ctrl+\ to execute and append to new result tab
    editor.addAction({
      id: 'execute-query-append',
      label: 'Execute to New Result Tab',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Backslash],
      contextMenuGroupId: 'execution',
      contextMenuOrder: 2,
      run: () => {
        executeQueryAppendRef.current?.();
      },
    });

    // Add Ctrl+Alt+F to format SQL
    editor.addAction({
      id: 'format-sql',
      label: 'Format SQL',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.KeyF],
      contextMenuGroupId: 'formatting',
      contextMenuOrder: 1,
      run: () => {
        formatQueryRef.current?.();
      },
    });

    // Add context menu action for executing selection
    editor.addAction({
      id: 'execute-selection',
      label: 'Execute Selection',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
      contextMenuGroupId: 'execution',
      contextMenuOrder: 3,
      precondition: 'editorHasSelection',
      run: () => {
        executeQueryRef.current?.();
      },
    });

    // Track selection changes to update execute button tooltip
    editor.onDidChangeCursorSelection((e) => {
      const selection = e.selection;
      setHasSelection(selection && !selection.isEmpty());
    });

    // Focus the editor
    editor.focus();
  }, []);

  // Handle result panel resize
  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isResizingResults) return;
      const container = document.querySelector('.query-editor-container');
      if (container) {
        const rect = container.getBoundingClientRect();
        const newHeight = rect.bottom - e.clientY;
        setResultPanelHeight(Math.max(100, Math.min(rect.height - 200, newHeight)));
      }
    };

    const handleMouseUp = () => {
      setIsResizingResults(false);
    };

    if (isResizingResults) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = 'row-resize';
      document.body.style.userSelect = 'none';
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [isResizingResults]);

  // Get active result from results array
  const activeResult = queryResults && queryResults.length > 0 ? queryResults[activeResultIndex] : null;
  const resultsHidden = useAppStore((state) => state.isResultsHidden(tab.id));
  const reorderQueryResults = useAppStore(state => state.reorderQueryResults);
  const showResults = (activeResult || isExecuting) && !resultsHidden;

  return (
    <div className="query-editor-container flex-1 flex flex-col min-h-0">
      <StickyNotesRenderer />
      {/* Arc-style editor toolbar */}
      <div className="flex items-center justify-between px-3 py-1.5 backdrop-blur-sm border-b border-[var(--border-color)]" style={{
        background: `linear-gradient(135deg, ${spaceColor}06 0%, transparent 50%, ${spaceColor}06 100%)`
      }}>
        <div className="flex items-center gap-2">
          {/* Tab icon with space color */}
          <div
            className="w-6 h-6 rounded-md flex items-center justify-center flex-shrink-0"
            style={{ backgroundColor: `${spaceColor}20` }}
          >
            <svg
              className="w-3.5 h-3.5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              style={{ color: spaceColor }}
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
            </svg>
          </div>

          <span className="text-sm font-medium text-[var(--text-primary)] truncate max-w-[200px]">
            {tab.title}
          </span>

          {tab.is_pinned && <span className="text-xs" title="Pinned">📌</span>}

          <span className="text-[10px] text-[var(--text-muted)]">•</span>

          <span className="text-[11px]">
            {isConnected ? (
              <span className="text-green-400">{activeSpace?.connection_username}@{activeSpace?.connection_host}</span>
            ) : hasConnection ? (
              <span className="text-amber-400">Disconnected</span>
            ) : (
              <span className="text-amber-400">No connection</span>
            )}
          </span>

          {/* Save indicator */}
          {isSaving && (
            <div className="flex items-center gap-1 px-1.5 py-0.5 rounded-full bg-[var(--bg-hover)] text-[var(--text-muted)] text-[10px]">
              <div className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
              Saving
            </div>
          )}
        </div>

        <div className="flex items-center gap-1">
          {/* Format button */}
          <button
            onClick={handleFormatSql}
            className="p-1.5 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
            title="Format SQL (Ctrl+Alt+F)"
          >
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h7" />
            </svg>
          </button>

          {/* Validation toggle button */}
          <button
            onClick={toggleValidation}
            className={`p-1.5 rounded-md transition-colors ${validationEnabled
              ? 'text-green-400 hover:text-green-300 bg-green-400/10 hover:bg-green-400/20'
              : 'text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]'
              }`}
            title={validationEnabled ? 'Validation enabled - click to disable' : 'Validation disabled - click to enable'}
          >
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              {validationEnabled ? (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
              ) : (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
              )}
            </svg>
          </button>

          {/* Run/Cancel button */}
          {isExecuting ? (
            <button
              onClick={() => cancelRunningQueries(tab.id)}
              className="px-2.5 py-1 text-white rounded-md text-xs font-medium flex items-center gap-1.5 transition-all hover:brightness-110 active:scale-95 bg-red-500"
              title="Cancel Query"
            >
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
              Cancel
            </button>
          ) : (
            <button
              onClick={handleExecuteQuery}
              disabled={!hasConnection}
              className="px-2.5 py-1 text-white rounded-md text-xs font-medium flex items-center gap-1.5 transition-all hover:brightness-110 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
              style={{
                backgroundColor: hasConnection ? spaceColor : '#666',
                boxShadow: hasConnection ? `0 1px 4px ${spaceColor}30` : 'none',
              }}
              title={
                !hasConnection
                  ? "Configure a connection first"
                  : hasSelection
                    ? "Execute selected text (Ctrl+Enter)"
                    : "Execute query (Ctrl+Enter)"
              }
            >
              <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M8 5v14l11-7z" />
              </svg>
              Run
            </button>
          )}
        </div>
      </div>

      {/* Monaco Editor */}
      <div className="flex-1 min-h-0 relative" style={{ flex: showResults ? '1 1 auto' : '1 1 100%' }} data-allow-select-all>
        <Editor
          height="100%"
          language="sql"
          theme={monacoTheme}
          defaultValue={tab.content || ''}
          onChange={handleChange}
          onMount={handleEditorMount}
          options={{
            fontSize: 12,
            fontFamily: "'Cascadia Code', 'Fira Code', 'JetBrains Mono', Consolas, monospace",
            fontLigatures: true,
            minimap: { enabled: false },
            lineNumbers: 'on',
            renderLineHighlight: 'line',
            scrollBeyondLastLine: false,
            wordWrap: 'on',
            automaticLayout: true,
            tabSize: 2,
            insertSpaces: true,
            padding: { top: 16, bottom: 16 },
            suggestOnTriggerCharacters: true,
            quickSuggestions: true,
            cursorBlinking: 'smooth',
            cursorSmoothCaretAnimation: 'on',
            smoothScrolling: true,
            scrollbar: {
              verticalScrollbarSize: 8,
              horizontalScrollbarSize: 8,
              useShadows: false,
            },
            overviewRulerBorder: false,
            hideCursorInOverviewRuler: true,
            renderLineHighlightOnlyWhenFocus: true,
            contextmenu: false, // Disable native Monaco context menu
          }}
        />

        {/* Custom Context Menu */}
        {contextMenu && (
          <ContextMenu
            items={contextMenuItems}
            position={contextMenu.position}
            onClose={() => setContextMenu(null)}
          />
        )}
      </div>

      {/* Results Panel with Virtual Scrolling */}
      {showResults && (
        <>
          {/* Resize handle */}
          <div
            className={`h-1 cursor-row-resize transition-colors flex-shrink-0 ${isResizingResults ? 'bg-[var(--accent-color)]' : 'bg-[var(--border-color)] hover:bg-[var(--border-subtle)]'
              }`}
            onMouseDown={() => setIsResizingResults(true)}
          />

          <div
            className="bg-[var(--bg-secondary)] border-t border-[var(--border-color)] flex flex-col overflow-hidden"
            style={{ height: resultPanelHeight }}
          >
            {/* Results header with horizontal tabs for multiple results */}
            {queryResults && queryResults.length > 1 ? (
              <div className="flex-shrink-0 border-b border-[var(--border-color)]">
                <DragDropContext onDragEnd={handleDragEnd}>
                  <Droppable droppableId={`results-tabs-${tab.id}`} direction="horizontal">
                    {(provided) => (
                      <div
                        ref={provided.innerRef}
                        {...provided.droppableProps}
                        className="flex items-center gap-1 px-2 py-1 bg-[var(--bg-tertiary)] overflow-x-auto no-scrollbar"
                      >
                        {queryResults.map((result, index) => {
                          const customName = getResultCustomName(tab.id, index);
                          const displayName = customName || `Result ${index + 1}`;
                          const isEditing = editingResultIndex === index;

                          return (
                            <Draggable key={`${tab.id}-result-${index}`} draggableId={`${tab.id}-result-${index}`} index={index}>
                              {(provided) => (
                                <div
                                  ref={provided.innerRef}
                                  {...provided.draggableProps}
                                  {...provided.dragHandleProps}
                                  className={`flex items-center gap-1 px-3 py-1.5 text-xs font-semibold rounded-t transition-all select-none ${index === activeResultIndex
                                    ? 'bg-[var(--bg-secondary)] text-[var(--text-primary)] shadow-md'
                                    : 'bg-[var(--bg-tertiary)] text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]'
                                    }`}
                                  style={{
                                    ...provided.draggableProps.style,
                                    ...(index === activeResultIndex
                                      ? {
                                        borderBottom: `2px solid ${spaceColor}`,
                                        boxShadow: `0 -2px 8px ${spaceColor}20, 0 2px 0 ${spaceColor}`
                                      }
                                      : {})
                                  }}
                                >
                                  {isEditing ? (
                                    <input
                                      type="text"
                                      value={editingResultName}
                                      onChange={(e) => setEditingResultName(e.target.value)}
                                      onBlur={() => {
                                        if (editingResultName.trim()) {
                                          setResultCustomName(tab.id, index, editingResultName.trim());
                                        }
                                        setEditingResultIndex(null);
                                      }}
                                      onKeyDown={(e) => {
                                        if (e.key === 'Enter') {
                                          if (editingResultName.trim()) {
                                            setResultCustomName(tab.id, index, editingResultName.trim());
                                          }
                                          setEditingResultIndex(null);
                                        } else if (e.key === 'Escape') {
                                          setEditingResultIndex(null);
                                        }
                                      }}
                                      autoFocus
                                      className="bg-[var(--bg-hover)] px-2 py-0.5 rounded text-xs outline-none focus:ring-1 focus:ring-[var(--accent-color)]"
                                      style={{ width: '120px' }}
                                      onMouseDown={(e) => e.stopPropagation()}
                                    />
                                  ) : (
                                    <div
                                      onClick={() => setActiveResultIndex(tab.id, index)}
                                      onDoubleClick={() => {
                                        setEditingResultIndex(index);
                                        setEditingResultName(customName || `Result ${index + 1}`);
                                      }}
                                      className="flex items-center gap-1.5 cursor-pointer"
                                    >
                                      <span>{displayName}</span>
                                      {result.row_count > 0 && (
                                        <span className={`${index === activeResultIndex ? 'opacity-70' : 'opacity-50'}`}>
                                          ({result.row_count.toLocaleString()})
                                        </span>
                                      )}
                                      {result.error && (
                                        <span className="text-red-400">✕</span>
                                      )}
                                    </div>
                                  )}
                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      closeResult(tab.id, index);
                                    }}
                                    className="ml-1 p-0.5 rounded hover:bg-[var(--bg-active)] transition-colors"
                                    title="Close this result"
                                    onMouseDown={(e) => e.stopPropagation()}
                                  >
                                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                                    </svg>
                                  </button>
                                </div>
                              )}
                            </Draggable>
                          );
                        })}
                        {provided.placeholder}
                      </div>
                    )}
                  </Droppable>
                </DragDropContext>
              </div>
            ) : (
              <div className="flex-shrink-0 border-b border-[var(--border-color)]">
                <div className="flex items-center justify-between px-4 py-2">
                  <div className="flex items-center gap-3">
                    {queryResults && queryResults.length <= 1 ? (
                      <>
                        <span className="text-sm font-medium text-[var(--text-primary)]">Results</span>
                        {activeResult && !activeResult.error && activeResult.columns.length > 0 && (
                          <span className="text-xs text-[var(--text-muted)]">
                            {activeResult.row_count.toLocaleString()} rows • {activeResult.execution_time_ms}ms
                          </span>
                        )}
                      </>
                    ) : null}
                    {isExecuting && (
                      <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
                        <div
                          className="w-3 h-3 border-2 border-t-transparent rounded-full animate-spin"
                          style={{ borderColor: `${spaceColor}60`, borderTopColor: 'transparent' }}
                        />
                        Executing...
                      </div>
                    )}
                  </div>
                  {!isExecuting && queryResults && queryResults.length === 1 && (
                    <button
                      onClick={() => clearQueryResult(tab.id)}
                      className="p-1 rounded hover:bg-[var(--bg-hover)] transition-colors text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                      title="Close result"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                      </svg>
                    </button>
                  )}
                </div>
              </div>
            )}

            {/* Results content with virtual scrolling */}
            <div className="flex-1 min-h-0 overflow-hidden">
              {activeResult ? (
                <ResultsGrid
                  result={activeResult}
                  onClose={() => clearQueryResult(tab.id)}
                  isExecuting={isExecuting}
                  spaceColor={spaceColor}
                  onExecuteUpdate={handleExecuteUpdateQuery}
                  canEdit={isConnected}
                  queryText={lastExecutedQuery ?? undefined}
                  tabId={tab.id}
                  resultIndex={activeResultIndex}
                />
              ) : isExecuting ? (
                <div className="flex items-center justify-center h-full">
                  <div className="flex items-center gap-3 text-[var(--text-muted)]">
                    <div
                      className="w-5 h-5 border-2 border-t-transparent rounded-full animate-spin"
                      style={{ borderColor: `${spaceColor}60`, borderTopColor: 'transparent' }}
                    />
                    Executing query...
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

export const QueryEditor = memo(QueryEditorComp);
