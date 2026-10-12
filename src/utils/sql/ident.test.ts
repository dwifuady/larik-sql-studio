import { describe, it, expect } from 'vitest';
import { quoteSqlIdent, quoteQualifiedSqlIdent, isSafePostgresBareIdent } from './ident';

describe('quoteSqlIdent', () => {
  it('brackets T-SQL identifiers and escapes ]', () => {
    expect(quoteSqlIdent('users', 'mssql')).toBe('[users]');
    expect(quoteSqlIdent('my]db', 'mssql')).toBe('[my]]db]');
  });

  it('leaves safe lowercase postgres identifiers bare', () => {
    expect(quoteSqlIdent('endorsement', 'postgres')).toBe('endorsement');
    expect(quoteSqlIdent('public', 'postgres')).toBe('public');
    expect(quoteSqlIdent('_x1$', 'postgres')).toBe('_x1$');
  });

  it('double-quotes unsafe or reserved postgres identifiers', () => {
    expect(quoteSqlIdent('MyTable', 'postgres')).toBe('"MyTable"');
    expect(quoteSqlIdent('order', 'postgres')).toBe('"order"');
    expect(quoteSqlIdent('select', 'postgres')).toBe('"select"');
    expect(quoteSqlIdent('a b', 'postgres')).toBe('"a b"');
    expect(quoteSqlIdent('1abc', 'postgres')).toBe('"1abc"');
    expect(quoteSqlIdent('we"ird', 'postgres')).toBe('"we""ird"');
  });

  it('qualifies schema.name pairs per dialect', () => {
    expect(quoteQualifiedSqlIdent('public', 'endorsement', 'postgres')).toBe('public.endorsement');
    expect(quoteQualifiedSqlIdent('public', 'endorsement', 'mssql')).toBe('[public].[endorsement]');
    expect(quoteQualifiedSqlIdent('MySchema', 'Order', 'postgres')).toBe('"MySchema"."Order"');
  });
});

describe('isSafePostgresBareIdent', () => {
  it('accepts bare-eligible names', () => {
    expect(isSafePostgresBareIdent('id')).toBe(true);
    expect(isSafePostgresBareIdent('created_at')).toBe(true);
  });

  it('rejects reserved words and unsafe shapes', () => {
    expect(isSafePostgresBareIdent('group')).toBe(false);
    expect(isSafePostgresBareIdent('UserId')).toBe(false);
    expect(isSafePostgresBareIdent('')).toBe(false);
    expect(isSafePostgresBareIdent('a;b')).toBe(false);
  });
});
