/**
 * T-SQL identifier quoting helpers (frontend).
 * Mirrors src-tauri/src/db/ident.rs for consistency.
 */

export function quoteIdent(name: string): string {
  return '[' + name.replace(/\]/g, ']]') + ']';
}

export function unwrapIdent(tok: string): string {
  if (tok.startsWith('[') && tok.endsWith(']')) {
    return tok.slice(1, -1).replace(/\]\]/g, ']');
  }
  return tok;
}
