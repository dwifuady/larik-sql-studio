# node-sql-parser Capabilities for SQL Server

## Summary

node-sql-parser (v5.4.0) provides good support for SQL Server syntax validation with the following findings:

## Supported Features

### SQL Server Specific Syntax
- ✅ **TOP clause**: Both `SELECT TOP 10` and `SELECT TOP (10)` work with `database: 'TransactSQL'`
- ✅ **Square brackets**: `[dbo].[Users]`, `[ColumnName]` - properly parsed with TransactSQL option
- ✅ **Standard SQL**: SELECT, INSERT, UPDATE, DELETE all work
- ✅ **JOINs**: INNER JOIN, LEFT JOIN with ON conditions
- ✅ **Subqueries**: Nested SELECT statements
- ✅ **CTEs**: WITH clauses (Common Table Expressions)
- ✅ **Multi-statement**: Multiple queries separated by semicolons return array of ASTs
- ✅ **Table aliases**: Properly tracked in AST structure

### Database Options
The parser supports multiple database dialects:
- `'TransactSQL'` - Best for SQL Server (handles TOP, square brackets)
- `'MySQL'` - Good fallback for standard SQL
- `'PostgreSQL'`, `'BigQuery'` - Other options available

**Recommendation**: Try `'TransactSQL'` first, fallback to `'MySQL'` if it fails.

## AST Structure

The parser returns well-structured AST with:
- `type`: Statement type ('select', 'insert', 'update', 'delete')
- `columns`: Array of column references with table info
- `from`: Array of table references with aliases
- `where`: Expression tree for WHERE clause
- `join`: Join information with ON conditions

### Column References
```json
{
  "type": "column_ref",
  "table": "u",
  "column": "FirstName",
  "collate": null
}
```

### Table References
```json
{
  "db": null,
  "table": "Users",
  "as": "u"
}
```

## Syntax Error Detection

The parser catches syntax errors and throws exceptions with descriptive messages:
- `SELECT * FORM Users` → "Expected '#', '--', '/*', 'SYSTEM_TIME', or [ \t\n\r] but 'M' found."
- `SELECT FROM Users` → Error about missing column list
- `SELECT * FROM` → Error about incomplete query

### Location Information

⚠️ **Limitation**: Error messages don't consistently include line/column numbers in a structured format. The error message text may contain position info but needs parsing.

**Strategy**: Extract location from error message text using regex, fallback to line 1, column 1 if not found.

## Limitations & Workarounds

### Known Limitations
1. **No built-in location tracking**: AST nodes don't have `line` and `column` properties by default
2. **GO batch separator**: Not SQL syntax, parser may not handle it (need to split manually)
3. **T-SQL specific features**: Some advanced SQL Server features may not be fully supported

### Workarounds
1. **Location tracking**: Use regex to extract from error messages, estimate from identifier names
2. **GO separator**: Pre-process queries to split on GO before parsing
3. **Unsupported syntax**: Wrap in try/catch, show generic syntax error

## Recommended Validation Approach

1. **Parse with TransactSQL first**
   ```typescript
   try {
     ast = parser.astify(query, { database: 'TransactSQL' });
   } catch (e) {
     // Try MySQL as fallback
     try {
       ast = parser.astify(query, { database: 'MySQL' });
     } catch (e2) {
       // Syntax error - show to user
     }
   }
   ```

2. **Walk AST to extract table/column references**
   - Use recursive function to visit all nodes
   - Collect table names from `from` array
   - Collect column names from `columns`, `where`, etc.

3. **Validate against schema**
   - Case-insensitive matching (SQL Server default)
   - Default to 'dbo' schema if not specified
   - Check table existence first, then column existence

4. **Generate errors with locations**
   - Use identifier length to calculate end position
   - Estimate line/column from query structure if needed

## Import Syntax

node-sql-parser uses CommonJS exports:

```typescript
// Correct import for ESM/TypeScript
import NodeSqlParser from 'node-sql-parser';
const { Parser } = NodeSqlParser;
const parser = new Parser();

// Or in CommonJS
const { Parser } = require('node-sql-parser');
```

## Test Results

All major SQL patterns tested successfully:
- ✅ Basic SELECT
- ✅ SELECT with TOP
- ✅ Square bracket identifiers
- ✅ INNER JOIN
- ✅ Subqueries
- ✅ CTEs (WITH clause)
- ✅ INSERT, UPDATE, DELETE
- ✅ Multi-statement queries
- ✅ Syntax error detection

## Next Steps

1. Create SqlValidator class using these findings
2. Implement AST walking to extract table/column references
3. Validate against SchemaInfo from Zustand store
4. Handle location extraction gracefully
5. Test with real-world queries from the application
