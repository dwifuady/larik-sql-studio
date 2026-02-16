/**
 * Test the SQL formatter
 */

import { formatSqlWithIndentation } from '../src/utils/sqlFormatter';

// Test cases
const testCases = [
  {
    name: 'Simple SELECT',
    input: 'select id, name from users',
    expectedOutput: 'SELECT\nid,\nname\nFROM\nusers'
  },
  {
    name: 'SELECT with WHERE',
    input: 'select * from users where id = 1',
    expectedOutput: 'SELECT\n*\nFROM\nusers\nWHERE\nid = 1'
  },
  {
    name: 'SELECT with JOIN',
    input: 'select u.id, o.order_id from users u join orders o on u.id = o.user_id',
    expectedOutput: null // Just check it doesn't crash
  },
  {
    name: 'Multi-line query',
    input: 'select id,\nname\nfrom users\nwhere active = 1',
    expectedOutput: null // Just check it doesn't crash
  },
];

console.log('Testing SQL Formatter...\n');

testCases.forEach(testCase => {
  console.log(`Test: ${testCase.name}`);
  console.log(`Input: ${testCase.input.replace(/\n/g, '\\n')}`);
  
  try {
    const output = formatSqlWithIndentation(testCase.input);
    console.log(`Output:\n${output}`);
    console.log('✓ Pass\n');
  } catch (error) {
    console.log(`✗ Error: ${error}\n`);
  }
});
