import { describe, it, expect } from 'vitest';
import { getResultStatementLabel } from './sql';

describe('getResultStatementLabel', () => {
  it('returns null for empty/nullish input', () => {
    expect(getResultStatementLabel(null)).toBeNull();
    expect(getResultStatementLabel(undefined)).toBeNull();
    expect(getResultStatementLabel('   ')).toBeNull();
  });

  it('extracts the table name from a SELECT ... FROM', () => {
    expect(getResultStatementLabel('SELECT * FROM [dbo].[AmendmentQuestion] AS [aq]')).toBe('AmendmentQuestion');
    expect(getResultStatementLabel('select name from sys.procedures where name = \'x\'')).toBe('procedures');
    expect(getResultStatementLabel('SELECT a.Id FROM Application a')).toBe('Application');
  });

  it('labels DML statements with verb + target', () => {
    expect(getResultStatementLabel('UPDATE bap SET x = 1')).toBe('UPDATE bap');
    expect(getResultStatementLabel('DELETE FROM [dbo].[Foo] WHERE Id = 1')).toBe('DELETE Foo');
    expect(getResultStatementLabel('INSERT INTO Orders (Id) VALUES (1)')).toBe('INSERT Orders');
  });

  it('labels EXEC statements', () => {
    expect(getResultStatementLabel('EXEC sp_PremiumFunding_ChangeToCreditCard')).toBe('EXEC sp_PremiumFunding_ChangeToCreditCard');
    expect(getResultStatementLabel('EXECUTE [dbo].[MyProc] @a = 1')).toBe('EXEC MyProc');
  });

  it('ignores leading comments when reading the verb', () => {
    expect(getResultStatementLabel('-- run this\nSELECT * FROM Users')).toBe('Users');
    expect(getResultStatementLabel('/* batch */ UPDATE t SET a = 1')).toBe('UPDATE t');
  });

  it('falls back to the first keyword for other statements', () => {
    expect(getResultStatementLabel('CREATE TABLE Foo (Id int)')).toBe('CREATE');
    expect(getResultStatementLabel('WITH cte AS (SELECT 1) SELECT * FROM cte')).toBe('cte');
  });
});
