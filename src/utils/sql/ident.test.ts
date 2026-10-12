import { describe, it, expect } from 'vitest';
import { quoteIdent, quotePostgresIdent, isSafePostgresBareIdent } from './ident';

describe('quoteIdent (T-SQL)', () => {
  it('brackets identifiers and escapes ]', () => {
    expect(quoteIdent('users')).toBe('[users]');
    expect(quoteIdent('my]db')).toBe('[my]]db]');
  });
});

describe('quotePostgresIdent', () => {
  it('leaves safe lowercase identifiers bare', () => {
    expect(quotePostgresIdent('endorsement')).toBe('endorsement');
    expect(quotePostgresIdent('public')).toBe('public');
    expect(quotePostgresIdent('_x1$')).toBe('_x1$');
  });

  it('double-quotes unsafe or reserved identifiers', () => {
    expect(quotePostgresIdent('MyTable')).toBe('"MyTable"');
    expect(quotePostgresIdent('order')).toBe('"order"');
    expect(quotePostgresIdent('select')).toBe('"select"');
    expect(quotePostgresIdent('a b')).toBe('"a b"');
    expect(quotePostgresIdent('1abc')).toBe('"1abc"');
    expect(quotePostgresIdent('we"ird')).toBe('"we""ird"');
  });
});

describe('isSafePostgresBareIdent', () => {
  it('accepts bare-eligible names', () => {
    expect(isSafePostgresBareIdent('id')).toBe(true);
    expect(isSafePostgresBareIdent('created_at')).toBe(true);
  });

  it('rejects reserved words and unsafe shapes', () => {
    expect(isSafePostgresBareIdent('group')).toBe(false);
    for (const word of ['window', 'cross', 'full', 'ilike', 'isnull', 'notnull', 'verbose', 'tablesample']) {
      expect(isSafePostgresBareIdent(word)).toBe(false);
    }
    expect(isSafePostgresBareIdent('UserId')).toBe(false);
    expect(isSafePostgresBareIdent('')).toBe(false);
    expect(isSafePostgresBareIdent('a;b')).toBe(false);
  });
});
