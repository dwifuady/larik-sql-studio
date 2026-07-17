
/**
 * Semantic color keys for sticky notes.
 * The actual hex value is resolved at render time based on the current theme
 * (dark or light), ensuring proper contrast in both modes.
 */
export type NoteColorKey = 'yellow' | 'blue' | 'green' | 'orange' | 'pink' | 'purple';

/**
 * Theme-aware color palettes.
 * Light mode: soft pastels with good contrast against dark text.
 * Dark mode: richer, more saturated tones that pop on dark backgrounds.
 */
export const NOTE_COLORS: Record<NoteColorKey, { light: string; dark: string }> = {
    yellow:  { light: '#fff9c4', dark: '#f59e0b' },
    blue:    { light: '#b3e5fc', dark: '#38bdf8' },
    green:   { light: '#c8e6c9', dark: '#22c55e' },
    orange:  { light: '#ffccbc', dark: '#f97316' },
    pink:    { light: '#f8bbd0', dark: '#ec4899' },
    purple:  { light: '#e1bee7', dark: '#a855f7' },
};

/** Ordered list of color keys for the color picker. */
export const NOTE_COLOR_KEYS: NoteColorKey[] = ['yellow', 'blue', 'green', 'orange', 'pink', 'purple'];

/**
 * Map a legacy hex color to the nearest semantic key.
 * This preserves backward compatibility with notes that stored raw hex values.
 */
const HEX_TO_KEY_MAP: Record<string, NoteColorKey> = {
    '#fff9c4': 'yellow',
    '#b3e5fc': 'blue',
    '#c8e6c9': 'green',
    '#ffccbc': 'orange',
    '#f8bbd0': 'pink',
    '#e1bee7': 'purple',
    // Dark-mode hexes (in case someone manually set them)
    '#f59e0b': 'yellow',
    '#38bdf8': 'blue',
    '#22c55e': 'green',
    '#f97316': 'orange',
    '#ec4899': 'pink',
    '#a855f7': 'purple',
};

/**
 * Normalise a stored color value to a semantic key.
 * Accepts either a known key or a legacy hex string.
 */
export function normalizeColorKey(color: string): NoteColorKey {
    if (NOTE_COLOR_KEYS.includes(color as NoteColorKey)) {
        return color as NoteColorKey;
    }
    const lower = color.toLowerCase();
    if (HEX_TO_KEY_MAP[lower]) return HEX_TO_KEY_MAP[lower];
    // Default to yellow for unknown colors
    return 'yellow';
}

/**
 * Resolve a color key to the actual hex for the given theme.
 */
export function resolveNoteColor(key: NoteColorKey, theme: 'dark' | 'light'): string {
    return NOTE_COLORS[key][theme];
}

export interface StickyNote {
    id: string;
    content: string;
    color: NoteColorKey;       // semantic key (theme-aware)
    minimized: boolean;
    lineNumber: number;        // 1-based, visual position
    originalLineNumber?: number;
}

export interface NoteData {
    id: string;
    content: string;
    color: string;             // stored as semantic key string for forward compat
    minimized?: boolean;
}

const NOTE_PREFIX = '-- @note: ';

/**
 * Parses the SQL content to find sticky notes.
 * Notes are stored as comments: -- @note: {"id":"...","content":"...","color":"yellow"}
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
                    id: noteData.id,
                    content: noteData.content,
                    color: normalizeColorKey(noteData.color),
                    minimized: !!noteData.minimized,
                    lineNumber: index + 1,
                    originalLineNumber: index + 1,
                });
            } catch (e) {
                console.warn('Failed to parse sticky note at line', index + 1, e);
            }
        }
    });

    return notes;
}

/**
 * Creates the serialized comment string for a note.
 * The color is stored as a semantic key string so it works in any theme.
 */
export function serializeNote(note: NoteData): string {
    return `${NOTE_PREFIX}${JSON.stringify(note)}`;
}

/**
 * Injects a note into the SQL content.
 * - If note exists (by ID), update it in place.
 * - If note is new, insert it at the specified line.
 */
export function injectNoteIntoSql(sql: string, note: StickyNote): string {
    const lines = sql.split('\n');
    const serialized = serializeNote({
        id: note.id,
        content: note.content,
        color: note.color,
        minimized: note.minimized,
    });

    // Check if updating existing note at same line
    if (note.originalLineNumber && lines[note.originalLineNumber - 1]?.trim().startsWith(NOTE_PREFIX)) {
        const existingLine = lines[note.originalLineNumber - 1];
        if (existingLine.includes(note.id)) {
            lines[note.originalLineNumber - 1] = serialized;
            return lines.join('\n');
        }
    }

    const insertIndex = Math.max(0, Math.min(lines.length, note.lineNumber - 1));
    lines.splice(insertIndex, 0, serialized);

    return lines.join('\n');
}

/**
 * Move a note to a different line in the SQL content.
 * Removes the note's comment from its current position and inserts it
 * at the target line. Other note lines are shifted accordingly.
 *
 * @param sql The full SQL content
 * @param noteId The ID of the note to move
 * @param targetLine 1-based line number where the note should be placed
 * @returns The updated SQL content
 */
export function moveNoteToLine(sql: string, noteId: string, targetLine: number): string {
    const lines = sql.split('\n');

    // Find and remove the note's current line
    let removedLine: string | null = null;
    let removedIndex = -1;
    for (let i = 0; i < lines.length; i++) {
        if (lines[i].trim().startsWith(NOTE_PREFIX) && lines[i].includes(noteId)) {
            removedLine = lines[i];
            removedIndex = i;
            lines.splice(i, 1);
            break;
        }
    }

    if (removedLine === null || removedIndex === -1) return sql;

    // Calculate the effective target index.
    // Since we removed one line, indices after removedIndex shifted by -1.
    let effectiveTarget = targetLine - 1; // convert to 0-based
    if (effectiveTarget > removedIndex) effectiveTarget -= 1; // account for removed line
    effectiveTarget = Math.max(0, Math.min(lines.length, effectiveTarget));

    // Insert the note at the new position
    lines.splice(effectiveTarget, 0, removedLine);

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
 * Generates a text representation of the notes for debugging or clipboard.
 */
export function notesToString(notes: StickyNote[]): string {
    return notes.map(n => `Line ${n.lineNumber}: ${n.content}`).join('\n');
}
