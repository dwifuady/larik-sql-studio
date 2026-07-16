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

// Helper function to split SQL batch into individual statements
// Handles semicolons, but avoids splitting inside strings or comments
export function splitSqlStatements(sql: string): string[] {
    const statements: string[] = [];
    let current = '';
    let inString = false;
    let stringChar = '';
    let inLineComment = false;
    let inBlockComment = false;

    for (let i = 0; i < sql.length; i++) {
        const char = sql[i];
        const nextChar = sql[i + 1] || '';

        // Handle line comments
        if (!inString && !inBlockComment && char === '-' && nextChar === '-') {
            inLineComment = true;
            current += char;
            continue;
        }
        if (inLineComment && (char === '\n' || char === '\r')) {
            inLineComment = false;
            current += char;
            continue;
        }

        // Handle block comments
        if (!inString && !inLineComment && char === '/' && nextChar === '*') {
            inBlockComment = true;
            current += char;
            continue;
        }
        if (inBlockComment && char === '*' && nextChar === '/') {
            inBlockComment = false;
            current += char + nextChar;
            i++;
            continue;
        }

        // Handle strings
        if (!inLineComment && !inBlockComment && (char === "'" || char === '"')) {
            if (!inString) {
                inString = true;
                stringChar = char;
            } else if (char === stringChar) {
                // Check for escaped quote (doubled)
                if (nextChar === stringChar) {
                    current += char + nextChar;
                    i++;
                    continue;
                }
                inString = false;
            }
        }

        // Handle semicolons (statement separator)
        if (!inString && !inLineComment && !inBlockComment && char === ';') {
            const trimmed = current.trim();
            if (trimmed.length > 0) {
                statements.push(trimmed);
            }
            current = '';
            continue;
        }

        current += char;
    }

    // Don't forget the last statement (may not have trailing semicolon)
    const trimmed = current.trim();
    if (trimmed.length > 0) {
        statements.push(trimmed);
    }

    return statements;
}
