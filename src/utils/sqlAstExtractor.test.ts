import { describe, it, expect } from 'vitest';
import { parseTableAliases, getCompletionContext } from './sqlAstExtractor';

describe('sqlAstExtractor CTE Support', () => {
    it('should extract CTE names from incomplete SQL using regex fallback', () => {
        const sql = `
            ;WITH cte_name AS (
                SELECT * FROM table
            )
            SELECT * FROM cte_n
        `;

        // This fails if AST parsing fails (due to incomplete SQL) and regex fallback misses CTEs
        const aliases = parseTableAliases(sql);

        expect(aliases.has('cte_name')).toBe(true);
        expect(aliases.get('cte_name')).toEqual({ schema: 'cte', table: 'cte_name' });
    });

    it('should extract explicit columns from CTE definition', () => {
        const sql = `
            ;WITH cte_columns AS (
                SELECT id as user_id, name FROM Users
            )
            SELECT * FROM cte_columns
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte_columns')).toBe(true);
        const info = aliases.get('cte_columns');
        expect(info?.columns).toBeDefined();
        expect(info?.columns).toContain('user_id');
        expect(info?.columns).toContain('name');
    });

    it('should extract source table from CTE when selecting *', () => {
        const sql = `
            ;WITH cte_star AS (
                SELECT * FROM Users
            )
            SELECT * FROM cte_star
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte_star')).toBe(true);
        const info = aliases.get('cte_star');
        expect(info?.sourceTable).toBeDefined();
        expect(info?.sourceTable?.table).toBe('Users');
    });

    it('should handle comma-separated CTEs', () => {
        const sql = `
            ;WITH cte1 AS (SELECT * FROM t1),
            cte2 AS (SELECT * FROM t2)
            SELECT * FROM cte2
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte1')).toBe(true);
        expect(aliases.has('cte2')).toBe(true);
    });

    it('should extract CTE columns even when main query is incomplete (causing AST failure)', () => {
        const sql = `
            ;WITH cte_failure AS (
                SELECT id, name FROM Users
            )
            SELECT * FROM cte_failure WHERE 
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte_failure')).toBe(true);
        const info = aliases.get('cte_failure');
        expect(info?.columns).toBeDefined();
        expect(info?.columns).toContain('id');
        expect(info?.columns).toContain('name');
    });

    it('should extract CTE source table even when main query is incomplete', () => {
        const sql = `
            ;WITH cte_source AS (
                SELECT * FROM Project
            )
            SELECT * FROM cte_source WHERE
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte_source')).toBe(true);
        const info = aliases.get('cte_source');
        expect(info?.sourceTable).toBeDefined();
        expect(info?.sourceTable?.table).toBe('Project');
    });
    it('should extract explicit columns from CTE definition syntax (col1, col2) AS', () => {
        // This query uses the syntax: CTE_Name (Col1, Col2) AS (...)
        // And is incomplete at the end to force regex fallback
        const sql = `
            ;WITH cte_explicit (AppId, Status) AS (
                SELECT Id, Status FROM Application
            )
            SELECT * FROM cte_explicit WHERE 
        `;

        const aliases = parseTableAliases(sql);

        expect(aliases.has('cte_explicit')).toBe(true);
        const info = aliases.get('cte_explicit');

        // This is expected to fail currently because regex fallback doesn't account for (AppId, Status)
        expect(info?.columns).toBeDefined();
        expect(info?.columns).toContain('AppId');
        expect(info?.columns).toContain('Status');
    });

    it('should handle explicit columns with newline', () => {
        const sql = `
            ;WITH cte_multiline 
            (
                ColA, 
                ColB
            ) 
            AS (
                SELECT 1, 2
            )
            SELECT * FROM cte_multiline
        `;

        const aliases = parseTableAliases(sql);
        expect(aliases.has('cte_multiline')).toBe(true);
        const info = aliases.get('cte_multiline');

        expect(info?.columns).toBeDefined();
        expect(info?.columns).toContain('ColA');
        expect(info?.columns).toContain('ColB');
    });
});

describe('sqlAstExtractor completion context', () => {
    it('keeps column context immediately after ON', () => {
        const textBeforeCursor = `
            SELECT *
            FROM [dbo].[Application] AS [a]
            JOIN [dbo].[BCApplicationQuotes] AS [bcaq] ON 
        `.trimEnd();

        const context = getCompletionContext(textBeforeCursor);

        expect(context.type).toBe('column');
        expect(context.lastKeyword).toBe('ON');
    });

    it('keeps column context when typing after AND in WHERE', () => {
        const textBeforeCursor = `
            SELECT q.[ApplicationId]
            FROM [dbo].[BCApplicationQuotes] q
            WHERE q.[ApplicationId] = 11347 AND q
        `.trimEnd();

        const context = getCompletionContext(textBeforeCursor);

        expect(context.type).toBe('column');
        expect(context.lastKeyword).toBe('AND');
        expect(context.partialWord).toBe('q');
    });

    it('keeps column context when typing after ORDER BY', () => {
        const textBeforeCursor = 'SELECT * FROM dbo.Product p ORDER BY na';
        const context = getCompletionContext(textBeforeCursor);

        expect(context.type).toBe('column');
        expect(context.lastKeyword).toBe('ORDER BY');
        expect(context.partialWord).toBe('na');
    });
});

describe('sqlAstExtractor alias resolution on large JOIN queries', () => {
    // Regression: when a SELECT-list column reference such as
    // `..., [f].[Reinstatements]\nFROM [dbo].[BCApplicationQuotes] q`
    // preceded the FROM keyword, pattern1's regex anchored on the comma,
    // crossed the newline with \s+, and captured the literal `FROM` keyword
    // as the alias. matchAll then advanced past FROM, so the real FROM-clause
    // alias (`q`) was never captured and `q.` produced no column suggestions.
    it('captures FROM-clause alias when a SELECT list ends with a bracketed column ref before the FROM line', () => {
        const sql = `SELECT q.[ApplicationId], q.[ProductId], p.[Name], [f].[ApplicationQuoteId], f.[CoverageSectionTypeId], cst.[Section], f.[LocationIndex], fi.[QuoteFeeItemTypeId], fit.[Shortname], fi.[Amount], fi.[IsProrated], [f].[Reinstatements]
FROM [dbo].[BCApplicationQuotes] q
JOIN [dbo].[BCApplicationQuoteFees] f ON [f].[ApplicationQuoteId] = [q].[Id] AND ISNULL(f.[IsDeleted], 0) = 0
JOIN [dbo].[BCApplicationQuoteFeeItem] fi ON [fi].[ApplicationQuoteId] = [q].[Id] AND [fi].[QuoteFeesId] = [f].[Id] AND ISNULL(fi.[IsDeleted], 0) = 0
LEFT JOIN [dbo].[CoverageSectionType] cst ON [cst].[SectionId] = [f].[CoverageSectionTypeId]
LEFT JOIN [dbo].[BCQuoteFeeItemType] fit ON [fit].[Id] = [fi].[QuoteFeeItemTypeId]
JOIN [dbo].[Product] p ON [p].[Id] = [q].[ProductId]
WHERE ISNULL(q.[IsDeleted], 0) = 0 AND q.`;

        const aliases = parseTableAliases(sql);

        expect(aliases.has('q')).toBe(true);
        expect(aliases.get('q')).toEqual({ schema: 'dbo', table: 'BCApplicationQuotes' });
        expect(aliases.has('f')).toBe(true);
        expect(aliases.has('fi')).toBe(true);
        expect(aliases.has('cst')).toBe(true);
        expect(aliases.has('fit')).toBe(true);
        expect(aliases.has('p')).toBe(true);
    });

    it('reports alias_column context for the trailing q. in a large JOIN WHERE clause', () => {
        const textBeforeCursor = `SELECT q.[ApplicationId], q.[ProductId], p.[Name], [f].[ApplicationQuoteId], f.[CoverageSectionTypeId], cst.[Section], f.[LocationIndex], fi.[QuoteFeeItemTypeId], fit.[Shortname], fi.[Amount], fi.[IsProrated], [f].[Reinstatements]
FROM [dbo].[BCApplicationQuotes] q
JOIN [dbo].[BCApplicationQuoteFees] f ON [f].[ApplicationQuoteId] = [q].[Id] AND ISNULL(f.[IsDeleted], 0) = 0
JOIN [dbo].[BCApplicationQuoteFeeItem] fi ON [fi].[ApplicationQuoteId] = [q].[Id] AND [fi].[QuoteFeesId] = [f].[Id] AND ISNULL(fi.[IsDeleted], 0) = 0
LEFT JOIN [dbo].[CoverageSectionType] cst ON [cst].[SectionId] = [f].[CoverageSectionTypeId]
LEFT JOIN [dbo].[BCQuoteFeeItemType] fit ON [fit].[Id] = [fi].[QuoteFeeItemTypeId]
JOIN [dbo].[Product] p ON [p].[Id] = [q].[ProductId]
WHERE ISNULL(q.[IsDeleted], 0) = 0 AND q.`;

        const context = getCompletionContext(textBeforeCursor, textBeforeCursor);

        expect(context.type).toBe('alias_column');
        if (context.type === 'alias_column') {
            expect(context.alias).toBe('q');
        }
    });
});
