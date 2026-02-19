import { describe, it, expect } from 'vitest';
import { parseTableAliases } from './sqlAstExtractor';

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
