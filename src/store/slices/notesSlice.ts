import { StateCreator } from 'zustand';
import * as notesApi from '../../api/notes';
import type { StickyNote } from '../../api/notes';
import type { AppState } from '../index';

// Debounce timers per note id — module-level so they survive across renders
const saveTimers: Record<string, ReturnType<typeof setTimeout>> = {};
const SAVE_DEBOUNCE_MS = 500;

export interface NotesSlice {
  // Cache of notes indexed by tab id
  notesByTab: Record<string, StickyNote[]>;
  notesLoading: Record<string, boolean>;
  notesError: string | null;

  // Actions
  fetchTabNotes: (tabId: string) => Promise<void>;
  addNote: (tabId: string, lineNumber: number) => Promise<StickyNote | null>;
  updateNote: (tabId: string, noteId: string, updates: Partial<StickyNote>) => void;
  removeNote: (tabId: string, noteId: string) => void;
  moveNoteInStore: (tabId: string, noteId: string, newLine: number) => void;
  clearTabNotesInStore: (tabId: string) => Promise<void>;
}

function generateId(): string {
  // Use crypto.randomUUID if available, fallback to a simple implementation
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return 'note-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 11);
}

function nowISO(): string {
  return new Date().toISOString();
}

export const createNotesSlice: StateCreator<AppState, [], [], NotesSlice> = (set, get) => ({
  notesByTab: {},
  notesLoading: {},
  notesError: null,

  fetchTabNotes: async (tabId: string) => {
    set((state) => ({
      notesLoading: { ...state.notesLoading, [tabId]: true },
    }));
    try {
      const notes = await notesApi.getTabNotes(tabId);
      set((state) => ({
        notesByTab: { ...state.notesByTab, [tabId]: notes },
        notesLoading: { ...state.notesLoading, [tabId]: false },
        notesError: null,
      }));
    } catch (err) {
      console.error('[NotesSlice] Failed to fetch tab notes:', err);
      set((state) => ({
        notesLoading: { ...state.notesLoading, [tabId]: false },
        notesError: err instanceof Error ? err.message : String(err),
      }));
    }
  },

  addNote: async (tabId: string, lineNumber: number) => {
    const newNote: StickyNote = {
      id: generateId(),
      tab_id: tabId,
      line_number: lineNumber,
      content: '',
      color: 'yellow',
      minimized: false,
      pinned: false,
      created_at: nowISO(),
      updated_at: nowISO(),
    };

    // Optimistic insert
    set((state) => ({
      notesByTab: {
        ...state.notesByTab,
        [tabId]: [...(state.notesByTab[tabId] ?? []), newNote].sort((a, b) => a.line_number - b.line_number),
      },
    }));

    // Persist
    try {
      await notesApi.saveNote(newNote);
      return newNote;
    } catch (err) {
      console.error('[NotesSlice] Failed to save new note:', err);
      // Revert on failure
      set((state) => ({
        notesByTab: {
          ...state.notesByTab,
          [tabId]: (state.notesByTab[tabId] ?? []).filter((n) => n.id !== newNote.id),
        },
        notesError: err instanceof Error ? err.message : String(err),
      }));
      return null;
    }
  },

  updateNote: (tabId: string, noteId: string, updates: Partial<StickyNote>) => {
    // Optimistic update
    set((state) => {
      const tabNotes = state.notesByTab[tabId] ?? [];
      const updatedNotes = tabNotes.map((n) =>
        n.id === noteId ? { ...n, ...updates, updated_at: nowISO() } : n
      );
      return {
        notesByTab: { ...state.notesByTab, [tabId]: updatedNotes },
      };
    });

    // Debounced DB write
    const existingTimer = saveTimers[noteId];
    if (existingTimer) clearTimeout(existingTimer);

    saveTimers[noteId] = setTimeout(async () => {
      delete saveTimers[noteId];
      const tabNotes = get().notesByTab[tabId] ?? [];
      const note = tabNotes.find((n) => n.id === noteId);
      if (!note) return;

      try {
        await notesApi.saveNote(note);
      } catch (err) {
        console.error('[NotesSlice] Failed to save note (debounced):', err);
        set({ notesError: err instanceof Error ? err.message : String(err) });
      }
    }, SAVE_DEBOUNCE_MS);
  },

  removeNote: (tabId: string, noteId: string) => {
    // Optimistic delete
    const removedNote: StickyNote | undefined = (get().notesByTab[tabId] ?? []).find((n) => n.id === noteId);

    set((state) => ({
      notesByTab: {
        ...state.notesByTab,
        [tabId]: (state.notesByTab[tabId] ?? []).filter((n) => n.id !== noteId),
      },
    }));

    // Cancel any pending save
    const timer = saveTimers[noteId];
    if (timer) {
      clearTimeout(timer);
      delete saveTimers[noteId];
    }

    // Persist
    if (removedNote) {
      notesApi.deleteNote(noteId, tabId).catch((err) => {
        console.error('[NotesSlice] Failed to delete note:', err);
        // Re-insert on failure
        set((state) => ({
          notesByTab: {
            ...state.notesByTab,
            [tabId]: [...(state.notesByTab[tabId] ?? []), removedNote].sort((a, b) => a.line_number - b.line_number),
          },
        }));
      });
    }
  },

  moveNoteInStore: (tabId: string, noteId: string, newLine: number) => {
    // Optimistic update
    set((state) => {
      const tabNotes = state.notesByTab[tabId] ?? [];
      const updatedNotes = tabNotes
        .map((n) => (n.id === noteId ? { ...n, line_number: newLine, updated_at: nowISO() } : n))
        .sort((a, b) => a.line_number - b.line_number);
      return {
        notesByTab: { ...state.notesByTab, [tabId]: updatedNotes },
      };
    });

    // Persist (not debounced — moves are infrequent)
    notesApi.moveNote(noteId, tabId, newLine).catch((err) => {
      console.error('[NotesSlice] Failed to move note:', err);
      set({ notesError: err instanceof Error ? err.message : String(err) });
    });
  },

  clearTabNotesInStore: async (tabId: string) => {
    const oldNotes = get().notesByTab[tabId] ?? [];

    // Optimistic clear
    set((state) => ({
      notesByTab: { ...state.notesByTab, [tabId]: [] },
    }));

    try {
      await notesApi.clearTabNotes(tabId);
    } catch (err) {
      console.error('[NotesSlice] Failed to clear tab notes:', err);
      // Restore on failure
      set((state) => ({
        notesByTab: { ...state.notesByTab, [tabId]: oldNotes },
        notesError: err instanceof Error ? err.message : String(err),
      }));
    }
  },
});
