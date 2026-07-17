// Legacy note utilities — kept for backward compatibility with export functionality.
//
// Sticky Notes v2 stores notes in the Rust SQLite database, NOT as inline
// SQL comments. The SQL editor content is always "clean" — no note markers
// are injected. Therefore `removeNotes` is now a passthrough that simply
// returns the input unchanged.
//
// This file exists so that the dynamic import() calls in useKeyboardShortcuts,
// CommandPalette, and TabsList continue to resolve without errors.

/**
 * Removes sticky note comments from SQL content.
 * In v2, notes are stored in the DB — SQL content is always clean.
 * This function is kept for backward compatibility and simply returns the input.
 */
export function removeNotes(sql: string): string {
  return sql;
}
