

export interface StickyNote {
    id: string;
    content: string;
    color: string;
    minimized: boolean;
    lineNumber: number; // 1-based, visual position
    originalLineNumber?: number; // Where it was found in the file
}

export interface NoteData {
    id: string;
    content: string;
    color: string;
    minimized?: boolean; // Optional for backward compatibility
}

const NOTE_PREFIX = '-- @note: ';

/**
 * Parses the SQL content to find sticky notes.
 * Notes are stored as comments: -- @note: {"id":"...","content":"...","color":"..."}
 */
export function extractNotes(sql: string): StickyNote[] {
    const notes: StickyNote[] = [];
    const lines = sql.split('\n');

    lines.forEach((line, index) => {
        const trimmed = line.trim();
        if (trimmed.startsWith(NOTE_PREFIX)) {
            try {
                const jsonStr = trimmed.substring(NOTE_PREFIX.length);
                const noteData = JSON.parse(jsonStr) as NoteData;

                notes.push({
                    ...noteData,
                    minimized: !!noteData.minimized,
                    lineNumber: index + 1, // 1-based
                    originalLineNumber: index + 1
                });
            } catch (e) {
                console.warn('Failed to parse sticky note at line', index + 1, e);
            }
        }
    });

    return notes;
}

/**
 * Creates the serialized comment string for a note
 */
export function serializeNote(note: NoteData): string {
    return `${NOTE_PREFIX}${JSON.stringify(note)}`;
}

/**
 * Injects a note into the SQL content.
 * logic:
 * - If note exists (by ID), update it.
 * - If note is new, insert it at the specified line.
 * 
 * Note: This simplified version assumes we are just appending/inserting in the editor
 * and letting the editor handle the text buffer. 
 * But for "Export" or "Save", we might need to process the whole string.
 */
export function injectNoteIntoSql(sql: string, note: StickyNote): string {
    const lines = sql.split('\n');
    const serialized = serializeNote({
        id: note.id,
        content: note.content,
        color: note.color,
        minimized: note.minimized
    });

    // Check if updating existing note at same line
    if (note.originalLineNumber && lines[note.originalLineNumber - 1]?.startsWith(NOTE_PREFIX)) {
        // Check if ID matches to be sure
        const existingLine = lines[note.originalLineNumber - 1];
        if (existingLine.includes(note.id)) {
            lines[note.originalLineNumber - 1] = serialized;
            return lines.join('\n');
        }
    }

    // Otherwise insert at the target line
    // Insert BEFORE the target line (so note appears above)
    // Or if lineNumber is meant to be the line OF the note

    // Strategy: The sticky note usually sits "above" a statement.
    // We'll insert it at the requested line index.
    const insertIndex = Math.max(0, Math.min(lines.length, note.lineNumber - 1));
    lines.splice(insertIndex, 0, serialized);

    return lines.join('\n');
}

/**
 * Removes all sticky note comments from the SQL.
 * Used for "Export Clean SQL".
 */
export function removeNotes(sql: string): string {
    return sql
        .split('\n')
        .filter(line => !line.trim().startsWith(NOTE_PREFIX))
        .join('\n');
}

/**
 * Generates a text representation of the notes for debugging or clipboard
 */
export function notesToString(notes: StickyNote[]): string {
    return notes.map(n => `Line ${n.lineNumber}: ${n.content}`).join('\n');
}
