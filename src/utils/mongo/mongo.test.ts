import { describe, it, expect } from 'vitest';
import {
  splitMongoStatements,
  findMongoStatementAtCursor,
  getMongoStatementLabel,
  mongoFindQuery,
  isMongoConnectionString,
  databaseFromConnectionString,
  isSqlStatement,
} from './shell';
import { getResultStatementLabel } from '../sql';
import { getMongoCompletionContext, buildMongoSuggestions, statementTextBeforeCursor } from './completion';
import type { SchemaInfo } from '../../types';
import { defaultDatabaseFor, spaceHasConnection } from '../../types';
import { connectionIsComplete } from '../../components/DriverPicker';

describe('splitMongoStatements', () => {
  it('splits on newlines before db/show/use and on semicolons', () => {
    const text = 'db.a.find()\ndb.b.find({ x: 1 });\n\nshow collections\nuse my-db\ndb.c.countDocuments()';
    expect(splitMongoStatements(text).map(s => s.statement)).toEqual([
      'db.a.find()',
      'db.b.find({ x: 1 })',
      'show collections',
      'use my-db',
      'db.c.countDocuments()',
    ]);
  });

  it('keeps chained calls on following lines together', () => {
    const text = 'db.users.find({})\n  .sort({ name: 1 })\n  .limit(5)\ndb.other.find()';
    const parts = splitMongoStatements(text);
    expect(parts).toHaveLength(2);
    expect(parts[0].statement).toBe('db.users.find({})\n  .sort({ name: 1 })\n  .limit(5)');
    // endColumn is exclusive, as Monaco expects
    expect(parts[0]).toMatchObject({ startLine: 1, startColumn: 1, endLine: 3, endColumn: 12 });
  });

  it('ignores db inside strings, comments, regexes and nested objects', () => {
    const text = "db.a.find({ s: 'x; db.b', db: 1, r: /db;/ }) // db.c.find()\ndb.d.find()";
    expect(splitMongoStatements(text).map(s => s.statement)).toEqual([
      "db.a.find({ s: 'x; db.b', db: 1, r: /db;/ })",
      'db.d.find()',
    ]);
  });

  it('does not split db.db.find()', () => {
    expect(splitMongoStatements('db.db.find()')).toHaveLength(1);
  });
});

describe('SQL statements in MongoDB spaces', () => {
  it('splits SQL and shell statements like the backend', () => {
    const text = "SELECT name, db FROM users -- db.x.find()\nWHERE age > 1\n  AND db = 'select'\ndb.users.find()\nselect count(*) from orders;\nSELECT 1 FROM t\nshow dbs";
    expect(splitMongoStatements(text).map(s => s.statement)).toEqual([
      "SELECT name, db FROM users -- db.x.find()\nWHERE age > 1\n  AND db = 'select'",
      'db.users.find()',
      'select count(*) from orders',
      'SELECT 1 FROM t',
      'show dbs',
    ]);
  });

  it('detects SQL after leading comments', () => {
    expect(isSqlStatement('  -- note\n  select * from t')).toBe(true);
    expect(isSqlStatement('/* x */ SELECT 1')).toBe(true);
    expect(isSqlStatement('db.selected.find()')).toBe(false);
  });

  it('labels SQL results with the SQL label', () => {
    expect(getMongoStatementLabel('SELECT * FROM users')).toBe(getResultStatementLabel('SELECT * FROM users'));
  });
});

describe('findMongoStatementAtCursor', () => {
  const text = 'db.a.find()\n\ndb.b.aggregate([\n  { $match: {} }\n])';

  it('finds the statement containing the cursor', () => {
    expect(findMongoStatementAtCursor(text, 4, 5)?.statement).toBe('db.b.aggregate([\n  { $match: {} }\n])');
  });

  it('uses the preceding statement when the cursor is between statements', () => {
    expect(findMongoStatementAtCursor(text, 2, 1)?.statement).toBe('db.a.find()');
  });

  it('returns null for empty text', () => {
    expect(findMongoStatementAtCursor('  \n', 1, 1)).toBeNull();
  });
});

describe('getMongoStatementLabel', () => {
  it.each([
    ['db.users.find({})', 'users.find'],
    ['db.getCollection("order-items").aggregate([])', 'order-items.aggregate'],
    ['db["x"].countDocuments()', 'x.countDocuments'],
    ['db.system.profile.find()', 'system.profile.find'],
    ['db.runCommand({ ping: 1 })', 'runCommand'],
    ['show dbs', 'show dbs'],
    ['use shop', 'use shop'],
  ])('%s → %s', (statement, label) => {
    expect(getMongoStatementLabel(statement)).toBe(label);
  });
});

describe('mongoFindQuery', () => {
  it('uses dot access for identifiers and getCollection otherwise', () => {
    expect(mongoFindQuery('users', 100)).toBe('db.users.find({}).limit(100)');
    expect(mongoFindQuery('order-items', 5)).toBe('db.getCollection("order-items").find({}).limit(5)');
  });
});

describe('connection strings', () => {
  it('detects mongodb:// and mongodb+srv:// hosts', () => {
    expect(isMongoConnectionString('mongodb://192.168.0.251:27017/')).toBe(true);
    expect(isMongoConnectionString(' MONGODB+SRV://cluster0.example.net')).toBe(true);
    expect(isMongoConnectionString('192.168.0.251')).toBe(false);
  });

  it('reads the database from the path', () => {
    expect(databaseFromConnectionString('mongodb://192.168.0.251:27017/')).toBeNull();
    expect(databaseFromConnectionString('mongodb://192.168.0.251:27017')).toBeNull();
    expect(databaseFromConnectionString('mongodb://u:p%40ss@h1,h2/shop?replicaSet=rs0')).toBe('shop');
    expect(databaseFromConnectionString('mongodb+srv://cluster0.example.net/my%20db')).toBe('my db');
  });
});

describe('getMongoCompletionContext', () => {
  it('offers statement keywords at the start', () => {
    expect(getMongoCompletionContext('')).toEqual({ kind: 'statement', prefix: '' });
    expect(getMongoCompletionContext('db.users.find()\nsh')).toEqual({ kind: 'statement', prefix: 'sh' });
  });

  it('offers collections after db.', () => {
    expect(getMongoCompletionContext('db.')).toEqual({ kind: 'collection', prefix: '' });
    expect(getMongoCompletionContext('db.us')).toEqual({ kind: 'collection', prefix: 'us' });
    expect(getMongoCompletionContext('db.getSiblingDB("x").')).toEqual({ kind: 'collection', prefix: '' });
  });

  it('offers collection names inside getCollection("…") and db["…"]', () => {
    expect(getMongoCompletionContext('db.getCollection("ord')).toEqual({ kind: 'collectionName', prefix: 'ord' });
    expect(getMongoCompletionContext('db["ord')).toEqual({ kind: 'collectionName', prefix: 'ord' });
  });

  it('offers collection methods after a collection', () => {
    expect(getMongoCompletionContext('db.users.fi')).toEqual({ kind: 'collectionMethod', collection: 'users', prefix: 'fi' });
    expect(getMongoCompletionContext('db.getCollection("order-items").')).toEqual({ kind: 'collectionMethod', collection: 'order-items', prefix: '' });
    expect(getMongoCompletionContext('db["a b"].')).toEqual({ kind: 'collectionMethod', collection: 'a b', prefix: '' });
  });

  it('offers cursor methods after a call', () => {
    expect(getMongoCompletionContext('db.users.find({ a: 1 }).so')).toEqual({
      kind: 'cursorMethod', collection: 'users', method: 'find', prefix: 'so',
    });
  });

  it('detects filter keys, operator keys and nesting', () => {
    expect(getMongoCompletionContext('db.users.find({ na')).toMatchObject({
      kind: 'key', collection: 'users', method: 'find', argIndex: 0, path: [], prefix: 'na',
    });
    expect(getMongoCompletionContext('db.users.find({ age: { $g')).toMatchObject({
      kind: 'key', path: ['age'], prefix: '$g',
    });
    expect(getMongoCompletionContext('db.users.find({ a: 1, $or: [ { ')).toMatchObject({
      kind: 'key', path: ['$or'],
    });
    expect(getMongoCompletionContext('db.users.find({}, { ')).toMatchObject({ kind: 'key', argIndex: 1 });
    expect(getMongoCompletionContext('db.users.find({}).sort({ ')).toMatchObject({ kind: 'key', method: 'sort' });
    expect(getMongoCompletionContext('db.users.find({ "addr')).toMatchObject({ kind: 'key', quoted: true, prefix: 'addr' });
  });

  it('detects pipeline stages and stage bodies', () => {
    expect(getMongoCompletionContext('db.o.aggregate([{ ')).toMatchObject({ kind: 'key', method: 'aggregate', path: [] });
    expect(getMongoCompletionContext('db.o.aggregate([{ $match: { st')).toMatchObject({ kind: 'key', path: ['$match'], prefix: 'st' });
    expect(getMongoCompletionContext('db.o.aggregate([{ $group: { _id: "$c", n: { ')).toMatchObject({ kind: 'key', path: ['$group', 'n'] });
    expect(getMongoCompletionContext('db.o.aggregate([{ $group: { _id: "$cu')).toEqual({ kind: 'fieldRef', collection: 'o', prefix: '$cu' });
    expect(getMongoCompletionContext('db.o.aggregate([{ $match: {} }, ')).toMatchObject({ kind: 'value', container: '[', path: [] });
  });

  it('detects value positions', () => {
    expect(getMongoCompletionContext('db.users.find({ _id: ')).toMatchObject({ kind: 'value', container: '{' });
    expect(getMongoCompletionContext('db.users.find(')).toMatchObject({ kind: 'value', container: '(' });
    expect(getMongoCompletionContext('db.users.find({ _id: ObjectId("')).toEqual({ kind: 'none' });
  });

  it('scopes to the statement being typed', () => {
    const text = 'db.a.find({ x: 1 })\ndb.users.find({ na';
    const before = statementTextBeforeCursor(text, 2, text.split('\n')[1].length + 1);
    expect(before).toBe('db.users.find({ na');
  });
});

describe('buildMongoSuggestions', () => {
  const schema: SchemaInfo = {
    database_name: 'shop',
    schemas: ['shop'],
    tables: [
      {
        schema_name: 'shop',
        table_name: 'users',
        table_type: 'COLLECTION',
        columns: [
          col('_id', 'objectId'),
          col('name', 'string'),
          col('address.city', 'string'),
        ],
      },
      { schema_name: 'shop', table_name: 'order-items', table_type: 'COLLECTION', columns: [] },
    ],
    relationships: [],
    routines: [],
    fetched_at: '',
  };

  const labels = (text: string) =>
    buildMongoSuggestions(getMongoCompletionContext(text), schema).map(s => s.label);

  it('suggests collections with getCollection for non-identifier names', () => {
    const items = buildMongoSuggestions(getMongoCompletionContext('db.'), schema);
    expect(items.find(i => i.label === 'users')?.insertText).toBe('users');
    expect(items.find(i => i.label === 'order-items')?.insertText).toBe('getCollection("order-items")');
    expect(items.map(i => i.label)).toContain('runCommand');
  });

  it('suggests fields and top-level operators in a filter', () => {
    const items = buildMongoSuggestions(getMongoCompletionContext('db.users.find({ '), schema);
    expect(items.map(i => i.label)).toEqual(expect.arrayContaining(['_id', 'name', 'address.city', '$or']));
    expect(items.find(i => i.label === 'address.city')?.insertText).toBe('"address.city"');
  });

  it('suggests comparison operators inside a field condition', () => {
    expect(labels('db.users.find({ age: { ')).toEqual(expect.arrayContaining(['$gt', '$in', '$exists']));
    expect(labels('db.users.find({ age: { ')).not.toContain('name');
  });

  it('suggests update operators, then fields inside them', () => {
    expect(labels('db.users.updateOne({}, { ')).toContain('$set');
    expect(labels('db.users.updateOne({}, { $set: { ')).toContain('name');
  });

  it('suggests stages, accumulators and $field references in pipelines', () => {
    expect(labels('db.users.aggregate([{ ')).toEqual(expect.arrayContaining(['$match', '$group', '$lookup']));
    expect(labels('db.users.aggregate([{ $group: { _id: null, n: { ')).toContain('$sum');
    // A quoted key is a key, not a $field reference
    expect(labels('db.users.aggregate([{ $project: { "$na')).not.toContain('$name');
    expect(labels('db.users.aggregate([{ $group: { _id: "$na')).toContain('$name');
  });

  it('suggests cursor methods only after find', () => {
    expect(labels('db.users.find().')).toContain('limit');
    expect(labels('db.users.aggregate([]).')).toEqual(['toArray']);
  });
});

function col(name: string, data_type: string) {
  return {
    name,
    data_type,
    max_length: null,
    precision: null,
    scale: null,
    is_nullable: false,
    is_primary_key: name === '_id',
    is_identity: false,
    is_computed: false,
    column_default: null,
    ordinal_position: 0,
  };
}

describe('optional MongoDB database', () => {
  const space = (driver: 'mssql' | 'mongodb', database: string | null) => ({
    id: 's', name: 'n', color: null, icon: null, created_at: '', updated_at: '', sort_order: 0,
    connection_host: '192.168.0.251', connection_port: 27017, connection_database: database,
    connection_username: null, connection_trust_cert: true, connection_encrypt: false,
    connection_driver: driver, last_active_tab_id: null,
  });
  const dbs = [
    { name: 'BlazeXHub', hasAccess: true },
    { name: 'admin', hasAccess: true },
    { name: 'secret', hasAccess: false },
  ];

  it('treats a MongoDB host alone as a connection', () => {
    expect(spaceHasConnection(space('mongodb', ''))).toBe(true);
    expect(spaceHasConnection(space('mssql', ''))).toBe(false);
    expect(connectionIsComplete({ driver: 'mongodb', host: '192.168.0.251', database: '' })).toBe(true);
    expect(connectionIsComplete({ driver: 'mssql', host: 'sql01', database: '' })).toBe(false);
  });

  it('defaults to the first accessible non-system database', () => {
    expect(defaultDatabaseFor(space('mongodb', ''), dbs)).toBe('BlazeXHub');
    expect(defaultDatabaseFor(space('mongodb', 'shop'), dbs)).toBe('shop');
    expect(defaultDatabaseFor(space('mongodb', ''), [{ name: 'admin', hasAccess: true }])).toBe('admin');
    expect(defaultDatabaseFor(space('mssql', ''), dbs)).toBeNull();
    expect(defaultDatabaseFor(space('mongodb', null), [])).toBeNull();
  });
});

describe('SQL completion in MongoDB spaces', () => {
  const schema: SchemaInfo = {
    database_name: 'shop',
    schemas: ['shop'],
    tables: [
      { schema_name: 'shop', table_name: 'users', table_type: 'COLLECTION', columns: [col('_id', 'objectId'), col('name', 'string'), col('address.city', 'string')] },
      { schema_name: 'shop', table_name: 'orders', table_type: 'COLLECTION', columns: [col('total', 'double'), col('unit price', 'double')] },
      { schema_name: 'shop', table_name: 'order-items', table_type: 'COLLECTION', columns: [] },
    ],
    relationships: [],
    routines: [],
    fetched_at: '',
  };
  const suggest = (before: string, full = before) =>
    buildMongoSuggestions(getMongoCompletionContext(before, full), schema);

  it('suggests collections after FROM and JOIN', () => {
    expect(getMongoCompletionContext('SELECT * FROM us')).toEqual({ kind: 'sqlCollection', prefix: 'us' });
    expect(getMongoCompletionContext('select * from users u join ')).toEqual({ kind: 'sqlCollection', prefix: '' });
    const items = suggest('SELECT * FROM ');
    expect(items.find(i => i.label === 'order-items')?.insertText).toBe('[order-items]');
  });

  it('suggests fields of the collections named anywhere in the statement', () => {
    const items = suggest('SELECT ', 'SELECT  FROM users');
    expect(items.map(i => i.label)).toEqual(expect.arrayContaining(['name', 'address.city', 'COUNT(*)', 'FROM']));
    expect(suggest('SELECT * FROM orders WHERE ').find(i => i.label === 'unit price')?.insertText).toBe('[unit price]');
  });

  it('resolves aliases to their collection', () => {
    const full = 'SELECT o.total, u. FROM orders o JOIN users u ON o.uid = u._id';
    const ctx = getMongoCompletionContext('SELECT o.total, u.', full);
    expect(ctx).toEqual({ kind: 'sqlField', collections: ['users'], prefix: '', qualified: true });
    expect(buildMongoSuggestions(ctx, schema).map(i => i.label)).toEqual(['_id', 'name', 'address.city']);
  });

  it('suggests clause keywords after a complete term and nothing inside strings', () => {
    expect(getMongoCompletionContext('SELECT * FROM users ')).toEqual({ kind: 'sqlKeyword', prefix: '' });
    expect(suggest('SELECT * FROM users WH').map(i => i.label)).toContain('WHERE');
    expect(getMongoCompletionContext("SELECT * FROM users WHERE name = 'jo")).toEqual({ kind: 'none' });
  });

  it('offers SELECT at the start of a statement', () => {
    expect(buildMongoSuggestions(getMongoCompletionContext(''), schema).map(i => i.label)).toContain('SELECT');
  });
});
