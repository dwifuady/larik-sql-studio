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
