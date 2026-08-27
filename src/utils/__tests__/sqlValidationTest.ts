/**
 * Test file for node-sql-parser SQL Server compatibility
 * This file is used to experiment with the parser and document capabilities/limitations
 */

// node-sql-parser uses CommonJS exports
import NodeSqlParser from 'node-sql-parser';

const { Parser } = NodeSqlParser;
const parser = new Parser();

// Test queries for SQL Server syntax
const testQueries = {
  // Basic SELECT
  basicSelect: `SELECT * FROM Users`,

  // SELECT with TOP
  selectTop: `SELECT TOP 10 * FROM Users`,
  selectTopWithParens: `SELECT TOP (10) * FROM Users`,

  // Square bracket identifiers
  squareBrackets: `SELECT [FirstName], [LastName] FROM [dbo].[Users]`,

  // JOIN queries
  innerJoin: `SELECT u.*, o.OrderId FROM Users u INNER JOIN Orders o ON u.UserId = o.UserId`,

  // Subqueries
  subquery: `SELECT * FROM (SELECT * FROM Users WHERE Active = 1) AS ActiveUsers`,

  // CTEs (WITH clause)
  cte: `WITH ActiveUsers AS (SELECT * FROM Users WHERE Active = 1) SELECT * FROM ActiveUsers`,

  // INSERT
  insert: `INSERT INTO Users (FirstName, LastName) VALUES ('John', 'Doe')`,

  // UPDATE
  update: `UPDATE Users SET Active = 1 WHERE UserId = 123`,

  // DELETE
  deleteQuery: `DELETE FROM Users WHERE UserId = 123`,

  // Multi-statement (separated by semicolon)
  multiStatement: `SELECT * FROM Users; SELECT * FROM Orders;`,

  // Syntax error examples
  syntaxError1: `SELECT * FORM Users`, // typo in FROM
  syntaxError2: `SELECT FROM Users`, // missing columns
  syntaxError3: `SELECT * FROM`, // incomplete
};

console.log('Testing node-sql-parser with SQL Server syntax...\n');

// Test each query
Object.entries(testQueries).forEach(([name, query]) => {
  console.log(`\n--- Testing: ${name} ---`);
  console.log(`Query: ${query.substring(0, 100)}${query.length > 100 ? '...' : ''}`);

  try {
    // Try different database options to see which works best
    const databases = ['MySQL', 'PostgreSQL', 'TransactSQL', 'MSSQL', 'BigQuery'];

    for (const db of databases) {
      try {
        const ast = parser.astify(query, { database: db as any });
        console.log(`✓ Success with database: ${db}`);
        console.log(`AST type:`, Array.isArray(ast) ? 'Array' : typeof ast);
        if (Array.isArray(ast)) {
          console.log(`AST length:`, ast.length);
          console.log(`First statement type:`, ast[0]?.type);
        } else {
          console.log(`AST statement type:`, (ast as any)?.type);
        }

        // Try to convert back to SQL
        try {
          const sql = parser.sqlify(ast, { database: db as any });
          console.log(`Sqlify result: ${sql.substring(0, 100)}${sql.length > 100 ? '...' : ''}`);
        } catch (sqlifyErr: any) {
          console.log(`Sqlify failed: ${sqlifyErr.message}`);
        }

        break; // Stop after first success
      } catch (dbErr: any) {
        // Try next database option
        if (db === databases[databases.length - 1]) {
          // Last option failed
          console.log(`✗ Failed with all database options`);
          console.log(`Last error: ${dbErr.message}`);
        }
      }
    }

  } catch (err: any) {
    console.log(`✗ Parse failed: ${err.message}`);
    // Try to extract location info if available
    if (err.location) {
      console.log(`  Location:`, err.location);
    }
  }
});

console.log('\n\n=== Testing AST Structure ===\n');

// Deep dive into AST structure for a simple query
const simpleQuery = `SELECT u.FirstName, u.LastName, o.OrderId
FROM Users u
INNER JOIN Orders o ON u.UserId = o.UserId
WHERE u.Active = 1`;

try {
  const ast = parser.astify(simpleQuery, { database: 'MySQL' });
  console.log('Simple query AST structure:');
  console.log(JSON.stringify(ast, null, 2));
} catch (err: any) {
  console.log('Failed to parse simple query:', err.message);
}

console.log('\n\n=== Capabilities Summary ===\n');
console.log('Run this file with: npx ts-node src/utils/sqlValidationTest.ts');
console.log('Or add to package.json scripts and run with: npm run test:sql-parser');
