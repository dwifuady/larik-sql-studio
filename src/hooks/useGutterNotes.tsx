import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import * as monaco from 'monaco-editor';
import type { editor } from 'monaco-editor';
import { useAppStore } from '../store';
import type { StickyNote } from '../api/notes';
import { NotePopover } from '../components/NotePopover';

type ITextModel = monaco.editor.ITextModel;
type IDisposable = monaco.IDisposable;

interface UseGutterNotesArgs {
  editor: editor.IStandaloneCodeEditor | null;
  model: ITextModel | null;
  tabId: string;
  enabled: boolean;
  theme: 'dark' | 'light';
}

interface ActivePopup {
  noteId: string;
  top: number;
  left: number;
}

export function useGutterNotes({ editor, model, tabId, enabled, theme }: UseGutterNotesArgs) {
  const [activePopup, setActivePopup] = useState<ActivePopup | null>(null);
  const decorationsRef = useRef<string[]>([]);
  const disposablesRef = useRef<IDisposable[]>([]);

  const notesByTab = useAppStore((s) => s.notesByTab);
  const fetchTabNotes = useAppStore((s) => s.fetchTabNotes);
  const addNote = useAppStore((s) => s.addNote);
  const updateNote = useAppStore((s) => s.updateNote);
  const removeNote = useAppStore((s) => s.removeNote);

  const notes: StickyNote[] = useMemo(() => notesByTab[tabId] ?? [], [notesByTab, tabId]);

  // Fetch notes from DB when tab changes
  useEffect(() => {
    if (!enabled || !tabId) return;
    fetchTabNotes(tabId);
  }, [tabId, enabled, fetchTabNotes]);

  // Sync decorations when notes change
  useEffect(() => {
    if (!editor || !model) return;

    if (!enabled) {
      if (decorationsRef.current.length > 0) {
        decorationsRef.current = editor.deltaDecorations(decorationsRef.current, []);
      }
      return;
    }

    const newDecorations: monaco.editor.IModelDeltaDecoration[] = notes.map((note) => ({
      range: new monaco.Range(note.line_number, 1, note.line_number, 1),
      options: {
        isWholeLine: false,
        glyphMarginClassName: 'larik-gutter-note-icon',
        glyphMarginHoverMessage: {
          value: note.content ? note.content.substring(0, 120) : 'Click to edit note',
        },
        stickiness: monaco.editor.TrackedRangeStickiness.GrowsOnlyWhenTypingAfter,
      },
    }));

    decorationsRef.current = editor.deltaDecorations(decorationsRef.current, newDecorations);
  }, [editor, model, notes, enabled]);

  // Gutter click handler + context menu + scroll listener
  useEffect(() => {
    if (!editor || !enabled) return;

    const mouseDownDisposable = editor.onMouseDown((e: editor.IEditorMouseEvent) => {
      if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN) {
        const line = e.target.position?.lineNumber;
        if (!line) return;

        // Check if a note exists at this line
        const existingNote = notes.find((n) => n.line_number === line);
        if (existingNote) {
          // Toggle popup for this note
          const pos = editor.getScrolledVisiblePosition({
            lineNumber: line,
            column: 1,
          });
          if (pos) {
            const editorDom = editor.getDomNode();
            const rect = editorDom?.getBoundingClientRect();
            if (rect) {
              setActivePopup({
                noteId: existingNote.id,
                top: rect.top + pos.top,
                left: rect.left + pos.left + 40, // offset past the gutter
              });
            }
          }
        } else {
          // No note at this line — could add one via context menu
        }
      }
    });

    // Close popup on scroll
    const scrollDisposable = editor.onDidScrollChange(() => {
      setActivePopup(null);
    });

    disposablesRef.current.push(mouseDownDisposable, scrollDisposable);

    return () => {
      mouseDownDisposable.dispose();
      scrollDisposable.dispose();
    };
  }, [editor, enabled, notes]);

  // Add note at a specific line
  const addNoteAtLine = useCallback(async (line: number) => {
    if (!enabled || !tabId) return;
    const newNote = await addNote(tabId, line);
    if (newNote && editor) {
      const pos = editor.getScrolledVisiblePosition({ lineNumber: line, column: 1 });
      if (pos) {
        const editorDom = editor.getDomNode();
        const rect = editorDom?.getBoundingClientRect();
        if (rect) {
          setActivePopup({
            noteId: newNote.id,
            top: rect.top + pos.top,
            left: rect.left + pos.left + 40,
          });
        }
      }
    }
  }, [enabled, tabId, addNote, editor]);

  // Handle note content/property changes from popup
  const handleNoteChange = useCallback((noteId: string, updates: Partial<StickyNote>) => {
    if (!tabId) return;
    updateNote(tabId, noteId, updates);
  }, [tabId, updateNote]);

  // Handle note deletion from popup
  const handleNoteDelete = useCallback((noteId: string) => {
    if (!tabId) return;
    removeNote(tabId, noteId);
  }, [tabId, removeNote]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      disposablesRef.current.forEach((d) => d.dispose());
      disposablesRef.current = [];
    };
  }, []);

  // The active popup note object
  const activeNote = activePopup ? notes.find((n) => n.id === activePopup.noteId) : null;

  // Renderer component for popups (rendered via portal-like fixed positioning)
  const GutterNotesRenderer = useCallback(() => {
    if (!activePopup || !activeNote) return null;
    return (
      <NotePopover
        note={activeNote}
        position={{ top: activePopup.top, left: activePopup.left }}
        theme={theme}
        onChange={handleNoteChange}
        onDelete={handleNoteDelete}
        onClose={() => setActivePopup(null)}
      />
    );
  }, [activePopup, activeNote, theme, handleNoteChange, handleNoteDelete]);

  return { GutterNotesRenderer, addNoteAtLine, notes };
}
