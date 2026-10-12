/**
 * Autocomplete for mongo-shell syntax.
 *
 * `getMongoCompletionContext` works out what the cursor is positioned on from
 * the text before it (collection name, method, filter key, pipeline stage,
 * "$field" reference, ...). `buildMongoSuggestions` turns that context plus the
 * sampled schema into editor-agnostic suggestions; QueryEditor maps them to
 * Monaco completion items.
 */
import type { SchemaInfo, SchemaColumnInfo } from '../../types';
import { splitMongoStatements, offsetAt, isSqlStatement } from './shell';

export type MongoCompletionContext =
  | { kind: 'statement'; prefix: string }
  | { kind: 'collection'; prefix: string }
  | { kind: 'collectionName'; prefix: string }
  | { kind: 'collectionMethod'; collection: string; prefix: string }
  | { kind: 'cursorMethod'; collection: string | null; method: string; prefix: string }
  | {
    kind: 'key';
    collection: string | null;
    /** Method whose arguments contain the cursor, e.g. `find` or `sort` */
    method: string;
    argIndex: number;
    /** Keys of the enclosing objects, outermost first */
    path: string[];
    /** Typing inside quotes: insert bare names */
    quoted: boolean;
    prefix: string;
  }
  | {
    kind: 'value';
    collection: string | null;
    method: string;
    argIndex: number;
    path: string[];
    /** Innermost container: `(` = a bare argument, `{` = a field value, `[` = an array element */
    container: '(' | '{' | '[';
    prefix: string;
  }
  | { kind: 'fieldRef'; collection: string | null; prefix: string }
  /** SQL SELECT: after FROM / JOIN */
  | { kind: 'sqlCollection'; prefix: string }
  /** SQL SELECT: an expression position (fields, functions, keywords) */
  | { kind: 'sqlField'; collections: string[]; prefix: string; qualified: boolean }
  /** SQL SELECT: after a complete term, where a clause keyword fits */
  | { kind: 'sqlKeyword'; prefix: string }
  | { kind: 'none' };

interface Frame {
  open: '(' | '{' | '[';
  /** `{`: key currently being written or whose value we're in */
  key: string | null;
  /** `{`: true right after `{` or `,` */
  expectingKey: boolean;
  /** `(`: index of the argument being written */
  argIndex: number;
  /** `(`: the function/method being called */
  callName: string | null;
}

interface ChainLink {
  name: string;
  called: boolean;
  /** First string argument, for getCollection("x") / getSiblingDB("x") / db["x"] */
  stringArg?: string;
  bracket?: boolean;
}

interface Scan {
  frames: Frame[];
  chain: ChainLink[];
  /** Unterminated string at the end: its quote and content so far */
  openString: { quote: string; content: string } | null;
  /** Identifier-ish characters immediately before the cursor */
  prefix: string;
  /** Character before the prefix (ignoring nothing) */
  charBeforePrefix: string;
  /** Last significant character before the prefix, skipping whitespace */
  lastSignificant: string;
  /** Whether a newline separates the prefix from the previous significant character */
  newlineBeforePrefix: boolean;
  startsWithDb: boolean;
}

function scan(text: string): Scan {
  const frames: Frame[] = [];
  const chain: ChainLink[] = [];
  let i = 0;
  let lastSignificant = '';
  let lastIdent = '';
  let startsWithDb = false;
  let seenToken = false;

  const top = () => frames[frames.length - 1];

  while (i < text.length) {
    const c = text[i];
    const next = text[i + 1];

    if (/\s/.test(c)) { i++; continue; }
    if (c === '/' && next === '/') {
      const nl = text.indexOf('\n', i);
      i = nl === -1 ? text.length : nl;
      continue;
    }
    if (c === '/' && next === '*') {
      const close = text.indexOf('*/', i + 2);
      if (close === -1) return emptyScan(text, frames, chain, startsWithDb);
      i = close + 2;
      continue;
    }

    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1;
      let content = '';
      let closed = false;
      while (j < text.length) {
        if (text[j] === '\\') { content += text[j + 1] ?? ''; j += 2; continue; }
        if (text[j] === c) { closed = true; break; }
        content += text[j];
        j++;
      }
      if (!closed) {
        return {
          frames, chain, openString: { quote: c, content },
          prefix: '', charBeforePrefix: c, lastSignificant, newlineBeforePrefix: false, startsWithDb,
        };
      }
      // String as key, value, or first argument
      const frame = top();
      if (frame?.open === '{' && frame.expectingKey) {
        frame.key = content;
      } else if (frame?.open === '(' && frames.length === 1 && frame.argIndex === 0) {
        const link = chain[chain.length - 1];
        if (link && link.stringArg === undefined) link.stringArg = content;
      } else if (frame?.open === '[' && frames.length === 1) {
        const link = chain[chain.length - 1];
        if (link?.bracket && link.stringArg === undefined) link.stringArg = content;
      }
      i = j + 1;
      lastSignificant = c;
      seenToken = true;
      continue;
    }

    if (/[A-Za-z_$]/.test(c)) {
      let j = i;
      while (j < text.length && /[\w$]/.test(text[j])) j++;
      const word = text.slice(i, j);
      if (!seenToken && word === 'db') startsWithDb = true;
      seenToken = true;
      const frame = top();
      if (frames.length === 0 && lastSignificant === '.') {
        chain.push({ name: word, called: false });
      } else if (frame?.open === '{' && frame.expectingKey) {
        frame.key = word;
      }
      lastIdent = word;
      i = j;
      lastSignificant = word[word.length - 1];
      continue;
    }

    seenToken = true;
    if (c === '(') {
      if (frames.length === 0 && chain.length > 0) chain[chain.length - 1].called = true;
      frames.push({ open: '(', key: null, expectingKey: false, argIndex: 0, callName: lastSignificant && /[\w$]/.test(lastSignificant) ? lastIdent : null });
    } else if (c === '{') {
      frames.push({ open: '{', key: null, expectingKey: true, argIndex: 0, callName: null });
    } else if (c === '[') {
      if (frames.length === 0 && startsWithDb) chain.push({ name: '[]', called: false, bracket: true });
      frames.push({ open: '[', key: null, expectingKey: false, argIndex: 0, callName: null });
    } else if (c === ')' || c === '}' || c === ']') {
      frames.pop();
      const frame = top();
      if (frame?.open === '{') frame.expectingKey = false;
    } else if (c === ':') {
      const frame = top();
      if (frame?.open === '{') frame.expectingKey = false;
    } else if (c === ',') {
      const frame = top();
      if (frame?.open === '{') { frame.expectingKey = true; frame.key = null; }
      if (frame?.open === '(') frame.argIndex++;
    }
    lastSignificant = c;
    i++;
  }

  return emptyScan(text, frames, chain, startsWithDb);
}

/** Final scan state when the text ends outside a string */
function emptyScan(text: string, frames: Frame[], chain: ChainLink[], startsWithDb: boolean): Scan {
  const prefixMatch = text.match(/[\w$.]*$/);
  let prefix = prefixMatch ? prefixMatch[0] : '';
  // A leading dot belongs to member access, not the prefix
  const dot = prefix.lastIndexOf('.');
  const keyLike = frames.length > 0 && frames[frames.length - 1].open === '{';
  if (!keyLike && dot >= 0) prefix = prefix.slice(dot + 1);
  const before = text.slice(0, text.length - prefix.length);
  const trimmed = before.replace(/\s+$/, '');
  // The in-progress word was pushed onto the chain by the scanner; drop it
  if (frames.length === 0 && prefix && chain.length > 0 && chain[chain.length - 1].name === prefix && !chain[chain.length - 1].called) {
    chain.pop();
  }
  return {
    frames,
    chain,
    openString: null,
    prefix,
    charBeforePrefix: before.slice(-1),
    lastSignificant: trimmed.slice(-1),
    newlineBeforePrefix: /\n\s*$/.test(before),
    startsWithDb,
  };
}

/** Resolve the collection, the first called method and a db-level call from the chain */
function interpretChain(chain: ChainLink[]): { collection: string | null; method: string | null; dbMethod: string | null } {
  const names: string[] = [];
  let collection: string | null = null;
  for (const link of chain) {
    if (link.bracket) { collection = link.stringArg ?? null; continue; }
    if (link.called) {
      if (link.name === 'getSiblingDB') continue;
      if (link.name === 'getCollection') { collection = link.stringArg ?? null; continue; }
      if (collection === null && names.length === 0) return { collection: null, method: null, dbMethod: link.name };
      return { collection: collection ?? names.join('.'), method: link.name, dbMethod: null };
    }
    names.push(link.name);
  }
  return { collection: collection ?? (names.length ? names.join('.') : null), method: null, dbMethod: null };
}

/** FROM / JOIN targets in a SQL statement, keyed by lowercase alias and name */
export function sqlCollections(statement: string): Map<string, string> {
  const found = new Map<string, string>();
  const pattern = /\b(?:FROM|JOIN)\s+(\[[^\]]+\]|"[^"]+"|`[^`]+`|[\w$.-]+)(?:\s+(?:AS\s+)?([A-Za-z_]\w*))?/gi;
  const reserved = /^(WHERE|GROUP|ORDER|HAVING|LIMIT|OFFSET|FETCH|JOIN|INNER|LEFT|RIGHT|FULL|CROSS|ON|UNION)$/i;
  for (const match of statement.matchAll(pattern)) {
    const name = match[1].replace(/^[["`]|[\]"`]$/g, '');
    found.set(name.toLowerCase(), name);
    if (match[2] && !reserved.test(match[2])) found.set(match[2].toLowerCase(), name);
  }
  return found;
}

const SQL_FIELD_POSITION = /(?:\b(?:SELECT|DISTINCT|WHERE|AND|OR|NOT|ON|BY|HAVING|WHEN|THEN|ELSE|CASE|IN|BETWEEN|LIKE|ILIKE|IS)|[,(=<>+\-*/%])$/i;

function sqlContext(textBeforeCursor: string, fullStatement: string): MongoCompletionContext {
  const tail = textBeforeCursor.match(/([\w$]+\.)?([\w$]*)$/);
  const qualifier = tail?.[1]?.slice(0, -1) ?? null;
  const prefix = tail?.[2] ?? '';
  const before = textBeforeCursor.slice(0, textBeforeCursor.length - (tail?.[0].length ?? 0)).replace(/\s+$/, '');
  const inString = ((before.match(/'/g) ?? []).length % 2) === 1;
  if (inString) return { kind: 'none' };

  if (/\b(?:FROM|JOIN)$/i.test(before) && !qualifier) return { kind: 'sqlCollection', prefix };

  const collections = sqlCollections(fullStatement);
  if (qualifier) {
    const target = collections.get(qualifier.toLowerCase());
    // `alias.` → that collection's fields; otherwise `address.` is a nested path
    return target
      ? { kind: 'sqlField', collections: [target], prefix, qualified: true }
      : { kind: 'sqlField', collections: [...new Set(collections.values())], prefix: `${qualifier}.${prefix}`, qualified: true };
  }
  if (!before || SQL_FIELD_POSITION.test(before)) {
    return { kind: 'sqlField', collections: [...new Set(collections.values())], prefix, qualified: false };
  }
  return { kind: 'sqlKeyword', prefix };
}

/**
 * Context for the cursor at the end of `textBeforeCursor` (the statement so far).
 * `fullStatement` (the whole statement, including text after the cursor) lets
 * SQL completions see FROM / JOIN clauses written later in the statement.
 */
export function getMongoCompletionContext(textBeforeCursor: string, fullStatement = textBeforeCursor): MongoCompletionContext {
  if (isSqlStatement(textBeforeCursor)) {
    return sqlContext(textBeforeCursor, fullStatement);
  }
  const s = scan(textBeforeCursor);
  const { collection, method } = interpretChain(s.chain);

  if (s.openString) {
    const frame = s.frames[s.frames.length - 1];
    const content = s.openString.content;
    if (s.frames.length === 1 && frame.open === '(' && frame.argIndex === 0 && frame.callName === 'getCollection') {
      return { kind: 'collectionName', prefix: content };
    }
    if (s.frames.length === 1 && frame.open === '[' && s.chain[s.chain.length - 1]?.bracket) {
      return { kind: 'collectionName', prefix: content };
    }
    if (frame?.open === '{' && frame.expectingKey) {
      return keyContext(s, collection, method, true, content);
    }
    if (content.startsWith('$') && frame?.open !== '(') {
      return { kind: 'fieldRef', collection, prefix: content };
    }
    return { kind: 'none' };
  }

  if (s.frames.length === 0) {
    if (s.charBeforePrefix === '.') {
      if (s.chain.length === 0) return s.startsWithDb ? { kind: 'collection', prefix: s.prefix } : { kind: 'none' };
      const last = s.chain[s.chain.length - 1];
      if (method) return { kind: 'cursorMethod', collection, method, prefix: s.prefix };
      if (last.called && last.name === 'getSiblingDB') return { kind: 'collection', prefix: s.prefix };
      if (collection) return { kind: 'collectionMethod', collection, prefix: s.prefix };
      return { kind: 'none' };
    }
    // Start of a statement: nothing yet, or a new line after a complete statement
    const before = textBeforeCursor.slice(0, textBeforeCursor.length - s.prefix.length);
    if (!before.trim() || (s.newlineBeforePrefix && s.lastSignificant !== '.')) {
      return { kind: 'statement', prefix: s.prefix };
    }
    return { kind: 'none' };
  }

  const frame = s.frames[s.frames.length - 1];
  if (frame.open === '{' && frame.expectingKey) {
    return keyContext(s, collection, method, false, s.prefix);
  }
  const call = callFrame(s.frames);
  const path = objectPath(s.frames);
  return {
    kind: 'value',
    collection,
    method: call?.name ?? method ?? '',
    argIndex: call?.argIndex ?? 0,
    path,
    container: frame.open,
    prefix: s.prefix,
  };
}

/** The outermost `(` frame: the method whose arguments we are in */
function callFrame(frames: Frame[]): { name: string; argIndex: number } | null {
  const first = frames[0];
  if (!first || first.open !== '(') return null;
  return { name: first.callName ?? '', argIndex: first.argIndex };
}

/** Keys of the enclosing objects (the key each nested container is the value of) */
function objectPath(frames: Frame[]): string[] {
  const path: string[] = [];
  // Every `{` frame except the innermost contributes its current key
  for (let idx = 0; idx < frames.length - 1; idx++) {
    const frame = frames[idx];
    if (frame.open === '{' && frame.key !== null) path.push(frame.key);
  }
  return path;
}

function keyContext(s: Scan, collection: string | null, chainMethod: string | null, quoted: boolean, prefix: string): MongoCompletionContext {
  const call = callFrame(s.frames);
  // A helper call nested in a value, e.g. ObjectId("..."), is not a key position
  const nestedCall = s.frames.slice(1).some(f => f.open === '(');
  if (nestedCall) return { kind: 'none' };
  return {
    kind: 'key',
    collection,
    method: call?.name ?? chainMethod ?? '',
    argIndex: call?.argIndex ?? 0,
    path: objectPath(s.frames),
    quoted,
    prefix,
  };
}

/** Statement text from its start up to the cursor (1-based Monaco position) */
export function statementTextBeforeCursor(fullText: string, line: number, column: number): string {
  const cursor = offsetAt(fullText, line, column);
  const statements = splitMongoStatements(fullText.slice(0, cursor));
  const current = statements[statements.length - 1];
  if (!current) return fullText.slice(0, cursor);
  // Text after the last complete statement belongs to the one being typed
  return fullText.slice(current.start, cursor);
}

// ============================================================================
// Suggestion catalogue
// ============================================================================

export type MongoSuggestionKind = 'keyword' | 'collection' | 'method' | 'field' | 'operator' | 'function' | 'snippet' | 'database';

export interface MongoSuggestion {
  label: string;
  kind: MongoSuggestionKind;
  insertText: string;
  /** insertText uses snippet placeholders ($1, ${1:x}) */
  isSnippet?: boolean;
  detail?: string;
  documentation?: string;
  sortText: string;
}

interface MethodSpec { name: string; snippet: string; doc: string }

export const COLLECTION_METHODS: MethodSpec[] = [
  { name: 'find', snippet: 'find({ $1 })$0', doc: 'find(filter?, projection?) — documents matching the filter. Chain .sort(), .limit(), .skip(), .projection().' },
  { name: 'findOne', snippet: 'findOne({ $1 })$0', doc: 'findOne(filter?, projection?) — the first matching document.' },
  { name: 'aggregate', snippet: 'aggregate([\n  { \\$match: { $1 } }\n])$0', doc: 'aggregate(pipeline) — run an aggregation pipeline.' },
  { name: 'countDocuments', snippet: 'countDocuments({ $1 })$0', doc: 'countDocuments(filter?) — exact count of matching documents.' },
  { name: 'estimatedDocumentCount', snippet: 'estimatedDocumentCount()$0', doc: 'estimatedDocumentCount() — fast count from collection metadata.' },
  { name: 'distinct', snippet: 'distinct("$1", { $2 })$0', doc: 'distinct(field, filter?) — distinct values of a field.' },
  { name: 'insertOne', snippet: 'insertOne({ $1 })$0', doc: 'insertOne(document) — insert one document.' },
  { name: 'insertMany', snippet: 'insertMany([\n  { $1 }\n])$0', doc: 'insertMany(documents) — insert several documents.' },
  { name: 'updateOne', snippet: 'updateOne({ $1 }, { \\$set: { $2 } })$0', doc: 'updateOne(filter, update, { upsert }?) — update the first matching document.' },
  { name: 'updateMany', snippet: 'updateMany({ $1 }, { \\$set: { $2 } })$0', doc: 'updateMany(filter, update, { upsert }?) — update every matching document.' },
  { name: 'replaceOne', snippet: 'replaceOne({ $1 }, { $2 })$0', doc: 'replaceOne(filter, replacement, { upsert }?) — replace a whole document.' },
  { name: 'deleteOne', snippet: 'deleteOne({ $1 })$0', doc: 'deleteOne(filter) — delete the first matching document.' },
  { name: 'deleteMany', snippet: 'deleteMany({ $1 })$0', doc: 'deleteMany(filter) — delete every matching document. Pass {} to delete all.' },
  { name: 'getIndexes', snippet: 'getIndexes()$0', doc: 'getIndexes() — list the indexes on this collection.' },
];

const FIND_CURSOR_METHODS: MethodSpec[] = [
  { name: 'sort', snippet: 'sort({ $1: ${2:1} })$0', doc: 'sort({ field: 1 | -1 })' },
  { name: 'limit', snippet: 'limit(${1:100})$0', doc: 'limit(n) — at most n documents.' },
  { name: 'skip', snippet: 'skip(${1:0})$0', doc: 'skip(n) — skip the first n documents.' },
  { name: 'projection', snippet: 'projection({ $1: 1 })$0', doc: 'projection({ field: 1 | 0 }) — choose returned fields.' },
  { name: 'count', snippet: 'count()$0', doc: 'count() — count matching documents instead of returning them.' },
  { name: 'maxTimeMS', snippet: 'maxTimeMS(${1:5000})$0', doc: 'maxTimeMS(ms) — abort on the server after ms milliseconds.' },
  { name: 'toArray', snippet: 'toArray()$0', doc: 'toArray() — accepted for compatibility; results always show in the grid.' },
];

const DATABASE_METHODS: MethodSpec[] = [
  { name: 'getCollection', snippet: 'getCollection("$1").$0', doc: 'getCollection(name) — a collection whose name is not a valid identifier.' },
  { name: 'getSiblingDB', snippet: 'getSiblingDB("$1").$0', doc: 'getSiblingDB(name) — run against another database.' },
  { name: 'runCommand', snippet: 'runCommand({ $1 })$0', doc: 'runCommand(command) — run a database command.' },
  { name: 'adminCommand', snippet: 'adminCommand({ $1 })$0', doc: 'adminCommand(command) — run a command against the admin database.' },
  { name: 'getCollectionNames', snippet: 'getCollectionNames()$0', doc: 'getCollectionNames() — list collections.' },
  { name: 'stats', snippet: 'stats()$0', doc: 'stats() — database statistics (dbStats).' },
];

const QUERY_FIELD_OPERATORS: [string, string][] = [
  ['$eq', 'Equal to'], ['$ne', 'Not equal to'], ['$gt', 'Greater than'], ['$gte', 'Greater than or equal'],
  ['$lt', 'Less than'], ['$lte', 'Less than or equal'], ['$in', 'In an array of values'], ['$nin', 'Not in an array of values'],
  ['$exists', 'Field exists (true/false)'], ['$type', 'BSON type'], ['$regex', 'Matches a regular expression'], ['$options', 'Regex options'],
  ['$elemMatch', 'An array element matches all conditions'], ['$size', 'Array has this many elements'], ['$all', 'Array contains all values'],
  ['$not', 'Negates an operator expression'], ['$mod', '[divisor, remainder]'],
];

const QUERY_TOP_OPERATORS: [string, string][] = [
  ['$and', 'All conditions match'], ['$or', 'Any condition matches'], ['$nor', 'No condition matches'],
  ['$expr', 'Use aggregation expressions'], ['$text', 'Text search'], ['$jsonSchema', 'Validate against a JSON Schema'], ['$comment', 'Attach a comment'],
];

const UPDATE_OPERATORS: [string, string][] = [
  ['$set', 'Set field values'], ['$unset', 'Remove fields'], ['$inc', 'Increment by an amount'], ['$mul', 'Multiply by an amount'],
  ['$rename', 'Rename fields'], ['$min', 'Update if lower'], ['$max', 'Update if higher'], ['$currentDate', 'Set to the current date'],
  ['$setOnInsert', 'Set only when upserting'], ['$push', 'Append to an array'], ['$pull', 'Remove matching array elements'],
  ['$pullAll', 'Remove listed array values'], ['$addToSet', 'Add to an array if absent'], ['$pop', 'Remove the first (-1) or last (1) element'],
];

const PIPELINE_STAGES: [string, string, string][] = [
  ['$match', 'Filter documents', '\\$match: { $1 }'],
  ['$project', 'Reshape documents', '\\$project: { $1 }'],
  ['$group', 'Group by a key', '\\$group: { _id: "\\$$1", ${2:count}: { \\$sum: 1 } }'],
  ['$sort', 'Sort documents', '\\$sort: { $1: ${2:1} }'],
  ['$limit', 'Limit the number of documents', '\\$limit: ${1:100}'],
  ['$skip', 'Skip documents', '\\$skip: ${1:0}'],
  ['$unwind', 'One document per array element', '\\$unwind: "\\$$1"'],
  ['$lookup', 'Join another collection', '\\$lookup: { from: "$1", localField: "$2", foreignField: "$3", as: "$4" }'],
  ['$addFields', 'Add computed fields', '\\$addFields: { $1 }'],
  ['$set', 'Add computed fields (alias of $addFields)', '\\$set: { $1 }'],
  ['$unset', 'Remove fields', '\\$unset: "$1"'],
  ['$count', 'Count documents into a field', '\\$count: "${1:count}"'],
  ['$facet', 'Run several sub-pipelines', '\\$facet: { $1: [] }'],
  ['$replaceRoot', 'Promote a sub-document', '\\$replaceRoot: { newRoot: "\\$$1" }'],
  ['$replaceWith', 'Replace the document', '\\$replaceWith: "\\$$1"'],
  ['$sample', 'Random sample', '\\$sample: { size: ${1:10} }'],
  ['$sortByCount', 'Group and count by a value', '\\$sortByCount: "\\$$1"'],
  ['$bucket', 'Group into boundary buckets', '\\$bucket: { groupBy: "\\$$1", boundaries: [$2], default: "Other" }'],
  ['$unionWith', 'Union with another collection', '\\$unionWith: "$1"'],
  ['$graphLookup', 'Recursive lookup', '\\$graphLookup: { from: "$1", startWith: "\\$$2", connectFromField: "$3", connectToField: "$4", as: "$5" }'],
  ['$out', 'Write results to a collection', '\\$out: "$1"'],
  ['$merge', 'Merge results into a collection', '\\$merge: { into: "$1" }'],
];

const ACCUMULATORS: [string, string][] = [
  ['$sum', 'Sum (use 1 to count)'], ['$avg', 'Average'], ['$min', 'Minimum'], ['$max', 'Maximum'],
  ['$first', 'First value in the group'], ['$last', 'Last value in the group'], ['$push', 'Array of values'],
  ['$addToSet', 'Array of distinct values'], ['$count', 'Count documents'], ['$stdDevPop', 'Population standard deviation'],
];

const EXPRESSION_OPERATORS: [string, string][] = [
  ['$concat', 'Concatenate strings'], ['$toUpper', 'Uppercase'], ['$toLower', 'Lowercase'], ['$substrCP', 'Substring'],
  ['$dateToString', 'Format a date'], ['$cond', 'If/then/else'], ['$ifNull', 'Fallback for null'], ['$size', 'Array length'],
  ['$arrayElemAt', 'Element at index'], ['$add', 'Add'], ['$subtract', 'Subtract'], ['$multiply', 'Multiply'], ['$divide', 'Divide'],
  ['$eq', 'Equal'], ['$ne', 'Not equal'], ['$gt', 'Greater than'], ['$gte', 'Greater or equal'], ['$lt', 'Less than'], ['$lte', 'Less or equal'],
  ['$and', 'Logical and'], ['$or', 'Logical or'], ['$not', 'Logical not'], ['$in', 'Value in array'], ['$toString', 'Convert to string'],
  ['$toInt', 'Convert to int'], ['$toDate', 'Convert to date'], ['$round', 'Round'], ['$filter', 'Filter an array'], ['$map', 'Map an array'],
];

const LOOKUP_KEYS: [string, string][] = [
  ['from', 'Collection to join'], ['localField', 'Field in this collection'], ['foreignField', 'Field in the joined collection'],
  ['as', 'Output array field'], ['let', 'Variables for the pipeline'], ['pipeline', 'Pipeline to run on the joined collection'],
];

const VALUE_HELPERS: [string, string, string][] = [
  ['ObjectId', 'ObjectId("$1")', 'ObjectId from 24 hex characters'],
  ['ISODate', 'ISODate("$1")', 'Date from an ISO 8601 string'],
  ['new Date', 'new Date("$1")', 'Date from an ISO 8601 string'],
  ['NumberLong', 'NumberLong($1)', '64-bit integer'],
  ['NumberInt', 'NumberInt($1)', '32-bit integer'],
  ['NumberDecimal', 'NumberDecimal("$1")', '128-bit decimal'],
  ['UUID', 'UUID("$1")', 'UUID (binary subtype 4)'],
];

const STATEMENT_KEYWORDS: [string, string, string][] = [
  ['db', 'db.', 'Current database'],
  ['SELECT', 'SELECT ', 'SQL query, run as an aggregation pipeline (read-only)'],
  ['show dbs', 'show dbs', 'List databases'],
  ['show collections', 'show collections', 'List collections'],
  ['use', 'use ', 'Switch the tab to another database'],
];

const SQL_CLAUSE_KEYWORDS = [
  'FROM', 'WHERE', 'JOIN', 'LEFT JOIN', 'ON', 'AND', 'OR', 'NOT', 'IN', 'LIKE', 'ILIKE', 'BETWEEN',
  'IS NULL', 'IS NOT NULL', 'AS', 'GROUP BY', 'HAVING', 'ORDER BY', 'ASC', 'DESC', 'LIMIT', 'OFFSET',
];

const SQL_FUNCTIONS: [string, string, string][] = [
  ['COUNT(*)', 'COUNT(*)', 'Number of rows'],
  ['COUNT', 'COUNT($1)', 'Non-null values'],
  ['SUM', 'SUM($1)', 'Sum'],
  ['AVG', 'AVG($1)', 'Average'],
  ['MIN', 'MIN($1)', 'Minimum'],
  ['MAX', 'MAX($1)', 'Maximum'],
  ['UPPER', 'UPPER($1)', 'Uppercase'],
  ['LOWER', 'LOWER($1)', 'Lowercase'],
  ['CONCAT', 'CONCAT($1)', 'Join strings (NULL counts as empty)'],
  ['COALESCE', 'COALESCE($1)', 'First non-null value'],
  ['LEN', 'LEN($1)', 'String length'],
  ['SUBSTRING', 'SUBSTRING($1, ${2:1}, ${3:10})', 'Substring (1-based start)'],
  ['ROUND', 'ROUND($1, ${2:2})', 'Round'],
  ['YEAR', 'YEAR($1)', 'Year of a date'],
  ['CAST', 'CAST($1 AS ${2:INT})', 'Convert type'],
  ['JSON_VALUE', "JSON_VALUE($1, '\\$.${2:key}')", 'Value at a JSON path, from JSON text or a sub-document'],
  ['ISODate', "ISODate('$1')", 'Date literal'],
  ['ObjectId', "ObjectId('$1')", 'ObjectId literal'],
];

/** SQL identifier, bracket-quoting segments that are not plain names */
function sqlIdentifier(name: string): string {
  return name.split('.').map(part => (/^[A-Za-z_][\w$]*$/.test(part) ? part : `[${part}]`)).join('.');
}

function sqlKeywordItems(sort: string): MongoSuggestion[] {
  return SQL_CLAUSE_KEYWORDS.map(k => ({ label: k, kind: 'keyword' as const, insertText: `${k} `, sortText: `${sort}_${k}` }));
}

/** Stages whose object keys are field names of the input documents */
const FIELD_KEY_STAGES = new Set(['$project', '$sort', '$addFields', '$set', '$unset', '$sortByCount']);
const FILTER_METHODS = new Set(['find', 'findOne', 'countDocuments', 'count', 'deleteOne', 'deleteMany', 'distinct']);
const LOGICAL_ARRAY_KEYS = new Set(['$and', '$or', '$nor']);
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;

function operatorItems(ops: [string, string][], sort: string): MongoSuggestion[] {
  return ops.map(([op, doc]) => ({
    label: op,
    kind: 'operator' as const,
    insertText: op,
    detail: doc,
    sortText: `${sort}_${op}`,
  }));
}

function fieldColumns(schema: SchemaInfo | null, collection: string | null): SchemaColumnInfo[] {
  if (!schema || !collection) return [];
  return schema.tables.find(t => t.table_name === collection)?.columns ?? [];
}

function fieldItems(columns: SchemaColumnInfo[], quoted: boolean): MongoSuggestion[] {
  return columns.map((col, index) => ({
    label: col.name,
    kind: 'field' as const,
    insertText: quoted || IDENTIFIER.test(col.name) ? col.name : `"${col.name}"`,
    detail: col.data_type,
    documentation: col.is_nullable ? 'Missing or null in some sampled documents' : undefined,
    sortText: `0_${String(index).padStart(4, '0')}`,
  }));
}

function methodItems(methods: MethodSpec[]): MongoSuggestion[] {
  return methods.map((m, index) => ({
    label: m.name,
    kind: 'method' as const,
    insertText: m.snippet,
    isSnippet: true,
    documentation: m.doc,
    sortText: `1_${String(index).padStart(2, '0')}`,
  }));
}

/** Keys valid in a filter (query) object at `path` */
function filterKeyItems(path: string[], columns: SchemaColumnInfo[], quoted: boolean): MongoSuggestion[] {
  const last = path[path.length - 1];
  // Inside `field: { ... }` → comparison operators
  if (last !== undefined && !last.startsWith('$')) {
    return operatorItems(QUERY_FIELD_OPERATORS, '0');
  }
  if (last === '$elemMatch') {
    return [...fieldItems(columns, quoted), ...operatorItems(QUERY_FIELD_OPERATORS, '2')];
  }
  if (last === '$expr') return operatorItems(EXPRESSION_OPERATORS, '0');
  return [...fieldItems(columns, quoted), ...operatorItems(QUERY_TOP_OPERATORS, '2')];
}

/** Path with logical-array keys ($or: [ {...} ]) removed, so nested filters resolve like top-level ones */
function stripLogical(path: string[]): string[] {
  let start = 0;
  path.forEach((key, idx) => { if (LOGICAL_ARRAY_KEYS.has(key)) start = idx + 1; });
  return path.slice(start);
}

function pipelineKeyItems(path: string[], columns: SchemaColumnInfo[], quoted: boolean): MongoSuggestion[] {
  const stage = path[0];
  if (stage === undefined) {
    return PIPELINE_STAGES.map(([name, doc, snippet], index) => ({
      label: name,
      kind: 'operator' as const,
      insertText: quoted ? name : snippet,
      isSnippet: !quoted,
      detail: doc,
      sortText: `0_${String(index).padStart(2, '0')}`,
    }));
  }
  const rest = path.slice(1);
  if (stage === '$match') return filterKeyItems(stripLogical(rest), columns, quoted);
  if (stage === '$group') {
    if (rest.length === 0) {
      return [{ label: '_id', kind: 'field', insertText: '_id', detail: 'Group key', sortText: '0__id' }];
    }
    return operatorItems(ACCUMULATORS, '0');
  }
  if (stage === '$lookup' && rest.length === 0) {
    return LOOKUP_KEYS.map(([key, doc]) => ({ label: key, kind: 'field' as const, insertText: key, detail: doc, sortText: `0_${key}` }));
  }
  if (FIELD_KEY_STAGES.has(stage)) {
    return rest.length === 0 ? fieldItems(columns, quoted) : operatorItems(EXPRESSION_OPERATORS, '0');
  }
  return operatorItems(EXPRESSION_OPERATORS, '1');
}

function keyItems(ctx: Extract<MongoCompletionContext, { kind: 'key' }>, columns: SchemaColumnInfo[]): MongoSuggestion[] {
  const { method, argIndex, path, quoted } = ctx;

  if (method === 'aggregate' && argIndex === 0) return pipelineKeyItems(path, columns, quoted);
  if ((method === 'updateOne' || method === 'updateMany') && argIndex === 1) {
    if (path.length === 0) return operatorItems(UPDATE_OPERATORS, '0');
    if (path.length === 1) return fieldItems(columns, quoted);
    return [];
  }
  if ((method === 'updateOne' || method === 'updateMany' || method === 'replaceOne') && argIndex === 2) {
    return [{ label: 'upsert', kind: 'field', insertText: 'upsert', detail: 'Insert when nothing matches', sortText: '0_upsert' }];
  }
  if (method === 'replaceOne' && argIndex === 1) return fieldItems(columns, quoted);
  if (method === 'insertOne' || method === 'insertMany') return path.length === 0 ? fieldItems(columns, quoted) : [];
  if (method === 'sort' || method === 'projection' || method === 'project') {
    return path.length === 0 ? fieldItems(columns, quoted) : [];
  }
  if ((method === 'find' || method === 'findOne') && argIndex === 1) {
    return path.length === 0 ? fieldItems(columns, quoted) : [];
  }
  const filterArg = method === 'distinct' ? 1 : 0;
  if ((FILTER_METHODS.has(method) || method === 'updateOne' || method === 'updateMany' || method === 'replaceOne') && argIndex === filterArg) {
    return filterKeyItems(stripLogical(path), columns, quoted);
  }
  return [];
}

function valueItems(ctx: Extract<MongoCompletionContext, { kind: 'value' }>): MongoSuggestion[] {
  if (ctx.method === 'aggregate' && ctx.argIndex === 0 && ctx.container === '[' && ctx.path.length === 0) {
    // Between stages: offer whole stage objects
    return PIPELINE_STAGES.map(([name, doc, snippet], index) => ({
      label: `{ ${name} }`,
      kind: 'snippet' as const,
      insertText: `{ ${snippet} }`,
      isSnippet: true,
      detail: doc,
      sortText: `0_${String(index).padStart(2, '0')}`,
    }));
  }
  if (ctx.container === '(') return []; // a bare argument: let the user type `{`
  return [
    ...VALUE_HELPERS.map(([label, snippet, doc]) => ({
      label,
      kind: 'function' as const,
      insertText: snippet,
      isSnippet: true,
      detail: doc,
      sortText: `1_${label}`,
    })),
    ...['true', 'false', 'null'].map(v => ({ label: v, kind: 'keyword' as const, insertText: v, sortText: `2_${v}` })),
  ];
}

/** Suggestions for a context, using the sampled schema for collection and field names */
export function buildMongoSuggestions(ctx: MongoCompletionContext, schema: SchemaInfo | null): MongoSuggestion[] {
  switch (ctx.kind) {
    case 'statement':
      return STATEMENT_KEYWORDS.map(([label, insertText, doc], index) => ({
        label, kind: 'keyword' as const, insertText, detail: doc, sortText: `0_${index}`,
      }));
    case 'collection': {
      const collections = (schema?.tables ?? []).map(t => ({
        label: t.table_name,
        kind: 'collection' as const,
        // Names that are not identifiers need getCollection("...")
        insertText: IDENTIFIER.test(t.table_name) ? t.table_name : `getCollection("${t.table_name}")`,
        detail: t.table_type === 'VIEW' ? 'view' : 'collection',
        sortText: `0_${t.table_name}`,
      }));
      return [...collections, ...methodItems(DATABASE_METHODS).map(m => ({ ...m, sortText: `5_${m.sortText}` }))];
    }
    case 'collectionName':
      return (schema?.tables ?? []).map(t => ({
        label: t.table_name, kind: 'collection' as const, insertText: t.table_name, detail: t.table_type === 'VIEW' ? 'view' : 'collection', sortText: `0_${t.table_name}`,
      }));
    case 'collectionMethod':
      return methodItems(COLLECTION_METHODS);
    case 'cursorMethod':
      if (ctx.method === 'find') return methodItems(FIND_CURSOR_METHODS);
      return methodItems(FIND_CURSOR_METHODS.filter(m => m.name === 'toArray'));
    case 'key':
      return keyItems(ctx, fieldColumns(schema, ctx.collection));
    case 'value':
      return valueItems(ctx);
    case 'fieldRef':
      return fieldColumns(schema, ctx.collection).map((col, index) => ({
        label: `$${col.name}`,
        kind: 'field' as const,
        insertText: `$${col.name}`,
        detail: col.data_type,
        sortText: `0_${String(index).padStart(4, '0')}`,
      }));
    case 'sqlCollection':
      return (schema?.tables ?? []).map(t => ({
        label: t.table_name,
        kind: 'collection' as const,
        insertText: sqlIdentifier(t.table_name),
        detail: t.table_type === 'VIEW' ? 'view' : 'collection',
        sortText: `0_${t.table_name}`,
      }));
    case 'sqlField': {
      const seen = new Set<string>();
      const fields: MongoSuggestion[] = [];
      for (const collection of ctx.collections) {
        fieldColumns(schema, collection).forEach((col, index) => {
          if (seen.has(col.name)) return;
          seen.add(col.name);
          fields.push({
            label: col.name,
            kind: 'field',
            insertText: sqlIdentifier(col.name),
            detail: ctx.collections.length > 1 ? `${col.data_type} · ${collection}` : col.data_type,
            sortText: `0_${String(index).padStart(4, '0')}`,
          });
        });
      }
      const functions = SQL_FUNCTIONS.map(([label, snippet, doc]) => ({
        label,
        kind: 'function' as const,
        insertText: snippet,
        isSnippet: true,
        detail: doc,
        sortText: `5_${label}`,
      }));
      // `alias.` and nested paths want fields only
      return ctx.qualified ? fields : [...fields, ...functions, ...sqlKeywordItems('8')];
    }
    case 'sqlKeyword':
      return sqlKeywordItems('0');
    case 'none':
      return [];
  }
}
