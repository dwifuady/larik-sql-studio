import { describe, it, expect } from 'vitest';
import {
  formatValueForInsert,
  coerceCellValue,
  buildUpdateQuery,
  buildDeleteQuery,
  buildInsertQuery,
} from './gridEdit';

describe('formatValueForInsert', () => {
    it('formats null as NULL', () => {
        expect(formatValueForInsert(null, 'int')).toBe('NULL');
    });

    it('formats booleans as 1/0', () => {
        expect(formatValueForInsert(true, 'bit')).toBe('1');
        expect(formatValueForInsert(false, 'bit')).toBe('0');
    });

    it('formats numbers bare', () => {
        expect(formatValueForInsert(42, 'int')).toBe('42');
    });

    it('formats binary arrays as hex literals', () => {
        expect(formatValueForInsert([0xde, 0xad], 'varbinary')).toBe('0xdead');
    });

    it('escapes single quotes in strings', () => {
        expect(formatValueForInsert("O'Brien", 'varchar')).toBe("'O''Brien'");
    });

    it('quotes date/time values', () => {
        expect(formatValueForInsert('2026-08-24', 'datetime')).toBe("'2026-08-24'");
    });

    it('prefixes unicode types with N', () => {
        expect(formatValueForInsert('héllo', 'nvarchar')).toBe("N'héllo'");
    });

    it('does not quote numeric strings for numeric columns', () => {
        expect(formatValueForInsert('42', 'decimal')).toBe('42');
    });

    it('quotes non-numeric strings for numeric columns as-is', () => {
        expect(formatValueForInsert('abc', 'decimal')).toBe("'abc'");
    });
});

describe('coerceCellValue', () => {
    it('maps empty string and "null" to null regardless of type', () => {
        expect(coerceCellValue('', 'int')).toBeNull();
        expect(coerceCellValue('null', 'varchar')).toBeNull();
        expect(coerceCellValue('NULL', 'datetime')).toBeNull();
    });

    it('parses numbers for numeric columns', () => {
        expect(coerceCellValue('42', 'int')).toBe(42);
        expect(coerceCellValue('3.5', 'decimal')).toBe(3.5);
    });

    it('keeps the raw string when numeric parsing fails', () => {
        expect(coerceCellValue('abc', 'int')).toBe('abc');
    });

    it('parses bit columns from 1/true', () => {
        expect(coerceCellValue('1', 'bit')).toBe(true);
        expect(coerceCellValue('true', 'bit')).toBe(true);
        expect(coerceCellValue('0', 'bit')).toBe(false);
    });

    it('passes through strings for text columns', () => {
        expect(coerceCellValue('hello', 'nvarchar')).toBe('hello');
    });
});

describe('buildUpdateQuery', () => {
    const identity = { name: 'id', dataType: 'int' };

    it('builds a single-column update', () => {
        expect(buildUpdateQuery('[dbo].[users]', identity, 7, [
            { column: { name: 'name', dataType: 'nvarchar' }, value: 'Ann' },
        ])).toBe("UPDATE [dbo].[users] SET [name] = N'Ann' WHERE [id] = 7");
    });

    it('builds a multi-column update with comma-separated sets', () => {
        expect(buildUpdateQuery('[dbo].[users]', identity, 7, [
            { column: { name: 'name', dataType: 'nvarchar' }, value: 'Ann' },
            { column: { name: 'age', dataType: 'int' }, value: null },
        ])).toBe("UPDATE [dbo].[users] SET [name] = N'Ann', [age] = NULL WHERE [id] = 7");
    });
});

describe('buildDeleteQuery', () => {
    it('targets one row by its key value', () => {
        expect(buildDeleteQuery('[dbo].[users]', { name: 'id', dataType: 'int' }, 12))
            .toBe('DELETE FROM [dbo].[users] WHERE [id] = 12');
    });

    it('handles string keys with quoting', () => {
        expect(buildDeleteQuery('[dbo].[codes]', { name: 'code', dataType: 'varchar' }, "A'1"))
            .toBe("DELETE FROM [dbo].[codes] WHERE [code] = 'A''1'");
    });
});

describe('buildInsertQuery', () => {
    it('inserts only the provided columns', () => {
        expect(buildInsertQuery(
            '[dbo].[users]',
            [{ name: 'name', dataType: 'nvarchar' }, { name: 'age', dataType: 'int' }],
            ['Bo', 30]
        )).toBe("INSERT INTO [dbo].[users] ([name], [age]) VALUES (N'Bo', 30)");
    });

    it('treats missing values as NULL', () => {
        expect(buildInsertQuery(
            '[dbo].[users]',
            [{ name: 'name', dataType: 'nvarchar' }],
            []
        )).toBe('INSERT INTO [dbo].[users] ([name]) VALUES (NULL)');
    });
});
