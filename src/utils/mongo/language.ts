/**
 * Monaco language definition for mongo-shell queries.
 *
 * A dedicated language id (rather than 'javascript') keeps Monaco's TypeScript
 * worker from offering browser globals and flagging shell helpers as errors.
 */
import type * as Monaco from 'monaco-editor';

export const MONGO_LANGUAGE_ID = 'mongodb';

const SHELL_HELPERS = [
  'ObjectId', 'ISODate', 'Date', 'NumberInt', 'NumberLong', 'NumberDecimal', 'Int32', 'Long',
  'Decimal128', 'Double', 'UUID', 'BinData', 'Timestamp', 'RegExp', 'MinKey', 'MaxKey',
];

// SQL SELECT is accepted too (compiled to an aggregation pipeline)
const SQL_KEYWORDS = [
  'select', 'distinct', 'top', 'from', 'where', 'join', 'inner', 'left', 'outer', 'on', 'and', 'or',
  'not', 'in', 'like', 'ilike', 'between', 'is', 'as', 'group', 'by', 'having', 'order', 'asc', 'desc',
  'limit', 'offset', 'fetch', 'next', 'first', 'rows', 'row', 'only', 'case', 'when', 'then', 'else',
  'end', 'cast',
];

const tokens: Monaco.languages.IMonarchLanguage = {
  defaultToken: '',
  tokenPostfix: '.mongodb',
  // SQL keywords are case-insensitive
  ignoreCase: true,
  keywords: ['db', 'show', 'use', 'new', ...SQL_KEYWORDS],
  constants: ['true', 'false', 'null', 'undefined', 'NaN', 'Infinity'],
  helpers: SHELL_HELPERS,
  tokenizer: {
    root: [
      [/\/\/.*$/, 'comment'],
      [/--.*$/, 'comment'],
      [/\/\*/, 'comment', '@comment'],
      // $operators and "$field" references stand out from plain keys
      [/\$[\w$]+/, 'keyword.operator'],
      [/[A-Za-z_][\w$]*/, {
        cases: {
          '@keywords': 'keyword',
          '@constants': 'constant',
          '@helpers': 'type.identifier',
          '@default': 'identifier',
        },
      }],
      [/"/, 'string', '@stringDouble'],
      [/'/, 'string', '@stringSingle'],
      [/`/, 'string', '@stringBacktick'],
      [/\/(?=([^\\/\n]|\\.)+\/[gimsuy]*)/, 'regexp', '@regexp'],
      [/0[xX][0-9a-fA-F]+/, 'number.hex'],
      [/-?\d*\.\d+([eE][-+]?\d+)?/, 'number.float'],
      [/-?\d+([eE][-+]?\d+)?/, 'number'],
      [/[{}()[\]]/, '@brackets'],
      [/[;,.:]/, 'delimiter'],
    ],
    comment: [
      [/[^/*]+/, 'comment'],
      [/\*\//, 'comment', '@pop'],
      [/[/*]/, 'comment'],
    ],
    stringDouble: [
      [/\$[\w.$]+/, 'keyword.operator'],
      [/[^\\"$]+/, 'string'],
      [/\\./, 'string.escape'],
      [/\$/, 'string'],
      [/"/, 'string', '@pop'],
    ],
    stringSingle: [
      [/\$[\w.$]+/, 'keyword.operator'],
      [/[^\\'$]+/, 'string'],
      [/\\./, 'string.escape'],
      [/\$/, 'string'],
      [/'/, 'string', '@pop'],
    ],
    stringBacktick: [
      [/[^\\`]+/, 'string'],
      [/\\./, 'string.escape'],
      [/`/, 'string', '@pop'],
    ],
    regexp: [
      [/\/[gimsuy]*/, 'regexp', '@pop'],
      [/\\./, 'regexp.escape'],
      [/[^\\/]+/, 'regexp'],
    ],
  },
};

const configuration: Monaco.languages.LanguageConfiguration = {
  comments: { lineComment: '//', blockComment: ['/*', '*/'] },
  brackets: [['{', '}'], ['[', ']'], ['(', ')']],
  autoClosingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: '"', close: '"', notIn: ['string'] },
    { open: "'", close: "'", notIn: ['string'] },
    { open: '`', close: '`', notIn: ['string'] },
  ],
  surroundingPairs: [
    { open: '{', close: '}' },
    { open: '[', close: ']' },
    { open: '(', close: ')' },
    { open: '"', close: '"' },
    { open: "'", close: "'" },
  ],
  // `$` is part of words so `$gte` selects and completes as one token
  wordPattern: /(-?\d*\.\d\w*)|([$\w][\w$]*)/g,
};

/** Register the language once per Monaco instance */
export function registerMongoLanguage(monaco: typeof Monaco): void {
  if (monaco.languages.getLanguages().some(l => l.id === MONGO_LANGUAGE_ID)) return;
  monaco.languages.register({ id: MONGO_LANGUAGE_ID, aliases: ['MongoDB', 'mongosh'] });
  monaco.languages.setMonarchTokensProvider(MONGO_LANGUAGE_ID, tokens);
  monaco.languages.setLanguageConfiguration(MONGO_LANGUAGE_ID, configuration);
}
