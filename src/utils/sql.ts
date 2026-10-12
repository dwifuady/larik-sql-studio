/**
 * Derive a short, human-friendly label for a result tab from the SQL statement
 * that produced it. Examples:
 *   "SELECT * FROM [dbo].[AmendmentQuestion]" -> "AmendmentQuestion"
 *   "UPDATE bap SET ..."                      -> "UPDATE bap"
 *   "SELECT name FROM sys.procedures ..."     -> "procedures"
 * Returns null when no meaningful label can be extracted (caller falls back to
 * the generic "Result N").
 */
export function getResultStatementLabel(statement: string | null | undefined): string | null {
    if (!statement) return null;

    // Strip leading line/block comments and whitespace so we read the real verb.
    const cleaned = statement
        .replace(/\/\*[\s\S]*?\*\//g, ' ')
        .replace(/--[^\n]*/g, ' ')
        .trim();
    if (!cleaned) return null;

    const unwrap = (raw: string): string => raw.replace(/[[\]"`]/g, '');
    // Table-ish reference: optional [schema]. + [name]
    const tableRef = '(?:\\[?[\\w]+\\]?\\.)?(\\[?[\\w]+\\]?)';

    // UPDATE / DELETE / INSERT INTO <target> -> "VERB target"
    // Checked before the generic FROM match so "DELETE FROM x" reads as DML.
    const dml = cleaned.match(new RegExp(`^(UPDATE|DELETE(?:\\s+FROM)?|INSERT\\s+INTO)\\s+${tableRef}`, 'i'));
    if (dml) {
        const verb = /^INSERT/i.test(dml[1]) ? 'INSERT' : /^DELETE/i.test(dml[1]) ? 'DELETE' : 'UPDATE';
        return `${verb} ${unwrap(dml[2])}`;
    }

    // SELECT ... FROM <table>  -> table name (the object being read)
    const fromMatch = cleaned.match(new RegExp(`\\bFROM\\s+${tableRef}`, 'i'));
    if (fromMatch) {
        return unwrap(fromMatch[1]);
    }

    // EXEC / EXECUTE <proc>
    const exec = cleaned.match(new RegExp(`^(?:EXEC|EXECUTE)\\s+${tableRef}`, 'i'));
    if (exec) {
        return `EXEC ${unwrap(exec[1])}`;
    }

    // Fallback: first keyword (CREATE, ALTER, WITH, DECLARE, ...)
    const firstWord = cleaned.match(/^([A-Za-z]+)/);
    return firstWord ? firstWord[1].toUpperCase() : null;
}

export { splitSqlStatements } from './sql/split';
