/**
 * SQL Server (T-SQL) dialect.
 */
import type { ParameterInfo, TableInfo } from '../types';
import { getDisplayDataType } from '../types';
import { quoteIdent } from '../utils/sql/ident';
import { splitSqlStatements } from '../utils/sql/split';
import type { SqlDialect } from './types';

const KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'IN', 'LIKE',
  'ORDER BY', 'GROUP BY', 'HAVING', 'JOIN', 'LEFT JOIN', 'RIGHT JOIN',
  'INNER JOIN', 'OUTER JOIN', 'FULL JOIN', 'CROSS JOIN', 'ON', 'AS', 'DISTINCT', 'TOP',
  'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET', 'DELETE',
  'CREATE', 'ALTER', 'DROP', 'TABLE', 'INDEX', 'VIEW', 'PROCEDURE', 'FUNCTION',
  'NULL', 'IS NULL', 'IS NOT NULL', 'BETWEEN', 'EXISTS',
  'UNION', 'UNION ALL', 'EXCEPT', 'INTERSECT',
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX', 'CASE', 'WHEN', 'THEN', 'ELSE', 'END',
  'CAST', 'CONVERT', 'COALESCE', 'NULLIF', 'IIF', 'ISNULL',
  'BEGIN', 'COMMIT', 'ROLLBACK', 'TRANSACTION',
  'DECLARE', 'EXEC', 'EXECUTE', 'RETURN', 'PRINT',
  'WITH', 'OVER', 'PARTITION BY', 'ROW_NUMBER', 'RANK', 'DENSE_RANK',
  'OFFSET', 'FETCH', 'NEXT', 'ROWS', 'ONLY',
  'ASC', 'DESC', 'NULLS', 'FIRST', 'LAST',
  'CROSS APPLY', 'OUTER APPLY', 'PIVOT', 'UNPIVOT',
  'MERGE', 'MATCHED', 'OUTPUT', 'INSERTED', 'DELETED',
  'USE',
];

const FUNCTIONS = [
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
  'ISNULL', 'ISNUMERIC', 'OBJECT_ID', 'OBJECT_NAME',
  'DB_ID', 'DB_NAME', 'SCHEMA_ID', 'SCHEMA_NAME', 'USER_ID', 'USER_NAME',
  // JSON functions
  'JSON_VALUE', 'JSON_QUERY', 'JSON_MODIFY', 'ISJSON', 'OPENJSON',
];

const JOIN_KEYWORDS = ['FROM', 'JOIN', 'INNER JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'FULL JOIN', 'OUTER JOIN', 'CROSS JOIN'];
const APPLY_KEYWORDS = ['APPLY', 'CROSS APPLY', 'OUTER APPLY'];

function quoteQualified(schema: string, name: string): string {
  return `${quoteIdent(schema)}.${quoteIdent(name)}`;
}

function objectReference(schema: string, name: string): string {
  return schema.toLowerCase() === 'dbo' ? name : quoteQualified(schema, name);
}

/** Input parameters, in call order. */
function inputParameters(parameters: ParameterInfo[]): ParameterInfo[] {
  return parameters.filter(p => p.parameter_mode !== 'OUT');
}

export const mssqlDialect: SqlDialect = {
  driver: 'mssql',
  monacoLanguage: 'sql',
  parserDatabases: ['TransactSQL', 'MySQL'],
  formatterLanguage: 'tsql',
  identifierQuote: 'bracket',
  defaultSchema: 'dbo',
  systemSchemas: ['dbo', 'sys', 'information_schema'],
  keywords: KEYWORDS,
  functions: FUNCTIONS,
  tableContextKeywords: [...JOIN_KEYWORDS, 'INTO', 'UPDATE', 'TABLE', ...APPLY_KEYWORDS],
  tableAliasKeywords: [...JOIN_KEYWORDS, ...APPLY_KEYWORDS],
  routineContextKeywords: ['EXEC', 'EXECUTE'],
  features: {
    useDatabase: true,
    localVariables: true,
    batchSeparator: true,
    semanticValidation: true,
    gridEdit: true,
    foreignKeyNavigation: true,
  },

  quoteIdent,
  quoteQualified,
  objectReference,
  quoteString: (value) => `N'${value.replace(/'/g, "''")}'`,
  splitStatements: splitSqlStatements,

  selectTopQuery: (schema, table, limit) =>
    `SELECT TOP ${limit} * FROM ${quoteQualified(schema, table)}`,

  // OBJECT_DEFINITION returns the full CREATE text of a view/proc/function.
  objectDefinitionQuery: (_kind, schema, name) =>
    `SELECT OBJECT_DEFINITION(OBJECT_ID(N'${quoteQualified(schema, name).replace(/'/g, "''")}')) AS [definition]`,

  createTableScript(table: TableInfo): string {
    const columns = [...table.columns].sort((a, b) => a.ordinal_position - b.ordinal_position);
    const columnLines = columns.map((col) => {
      const parts = [`    ${quoteIdent(col.name)}`, getDisplayDataType(col)];
      if (col.is_identity) parts.push('IDENTITY(1,1)');
      parts.push(col.is_nullable ? 'NULL' : 'NOT NULL');
      if (col.column_default) parts.push(`DEFAULT ${col.column_default}`);
      return parts.join(' ');
    });
    const pkColumns = columns.filter((c) => c.is_primary_key).map((c) => quoteIdent(c.name));
    if (pkColumns.length > 0) {
      columnLines.push(`    CONSTRAINT ${quoteIdent(`PK_${table.table_name}`)} PRIMARY KEY (${pkColumns.join(', ')})`);
    }
    return `CREATE TABLE ${quoteQualified(table.schema_name, table.table_name)} (\n${columnLines.join(',\n')}\n);`;
  },

  // Named parameters: `proc @a = ${1:NULL}, @b = ${2:NULL}`
  procedureCallSnippet(schema, name, parameters) {
    const args = inputParameters(parameters)
      .map((param, index) => `${param.name} = \${${index + 1}:NULL}`)
      .join(', ');
    const target = objectReference(schema, name);
    return args ? `${target} ${args}` : target;
  },

  // Scalar UDFs must be schema-qualified in T-SQL.
  functionCallSnippet(schema, name, parameters) {
    const args = inputParameters(parameters)
      .map((_, index) => `\${${index + 1}:NULL}`)
      .join(', ');
    return `${quoteQualified(schema, name)}(${args})`;
  },
};
