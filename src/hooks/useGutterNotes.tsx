import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { createPortal } from 'react-dom';
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
  zIndex: number;
}

/**
 * Compute the pixel offset from the editor's left edge to where the
 * glyph-margin icon is rendered. We measure the actual Monaco margin
 * DOM so the popover sits right next to the icon instead of using a
 * hardcoded 40 px.
 */
function getGlyphMarginOffset(editor: editor.IStandaloneCodeEditor): number {
  const dom = editor.getDomNode();
  if (!dom) return 0;
  const editorRect = dom.getBoundingClientRect();
  // The margin-view-overlays contains line numbers + glyph margin.
  // We want the right edge of the glyph margin specifically, which is
  // the left portion of the margin view. Measure the glyph margin element
  // directly, or fall back to the margin overlay width.
  const glyphMargin = dom.querySelector('.glyph-margin') as HTMLElement | null;
  if (glyphMargin) {
    const glyphRect = glyphMargin.getBoundingClientRect();
    return glyphRect.right - editorRect.left;
  }
  // Fallback: use the full margin overlay width
  const margin = dom.querySelector('.margin-view-overlays') as HTMLElement | null;
  if (margin) {
    return margin.getBoundingClientRect().right - editorRect.left;
  }
  return 0;
}

export function useGutterNotes({ editor, model, tabId, enabled, theme }: UseGutterNotesArgs) {
  // Multiple popovers can be open at once — pinned notes coexist; only one
  // unpinned note is open at a time (new unpinned opens close other unpinned).
  // `nextZ` starts above the pinned baseline; each bring-to-front bumps it.
  const BASE_Z = 1000;
  const [activePopups, setActivePopups] = useState<ActivePopup[]>([]);
  const nextZRef = useRef(BASE_Z + 1);

  const decorationsRef = useRef<string[]>([]);
  const hoverDecorationRef = useRef<string[]>([]);
  const disposablesRef = useRef<IDisposable[]>([]);

  const notesByTab = useAppStore((s) => s.notesByTab);
  const fetchTabNotes = useAppStore((s) => s.fetchTabNotes);
  const addNote = useAppStore((s) => s.addNote);
  const updateNote = useAppStore((s) => s.updateNote);
  const removeNote = useAppStore((s) => s.removeNote);

  const notes: StickyNote[] = useMemo(() => notesByTab[tabId] ?? [], [notesByTab, tabId]);

  // ── Refs for stable event handlers ──────────────────────────────────────
  // These let us keep Monaco event listeners attached without re-creating
  // them on every `notes` change. Handlers read `.current` at call-time.
  const notesRef = useRef(notes);
  notesRef.current = notes;

  // Helper: open or focus a note. Closes other unpinned notes if the newly
  // opened note is itself unpinned (matches the prior single-popup behavior
  // for the unpinned case, while pinned notes stack independently).
  const openOrFocusPopup = useCallback((noteId: string, top: number, left: number) => {
    setActivePopups((current) => {
      const existing = current.find((p) => p.noteId === noteId);
      const z = ++nextZRef.current;
      if (existing) {
        // Bring to front.
        return current.map((p) => (p.noteId === noteId ? { ...p, top, left, zIndex: z } : p));
      }
      // Look up the note to decide coexistence.
      const note = notesRef.current.find((n) => n.id === noteId);
      const newPopup: ActivePopup = { noteId, top, left, zIndex: z };
      if (note?.pinned) {
        return [...current, newPopup];
      }
      // New unpinned popover — replace any other unpinned popovers.
      return [...current.filter((p) => {
        const n = notesRef.current.find((nn) => nn.id === p.noteId);
        return n?.pinned;
      }), newPopup];
    });
  }, []);

  const addNoteAtLineRef = useRef<(line: number) => Promise<void>>(async () => {});
  const addNoteAtLine = useCallback(async (line: number) => {
    if (!enabled || !tabId) return;
    const newNote = await addNote(tabId, line);
    if (newNote && editor) {
      const pos = editor.getScrolledVisiblePosition({ lineNumber: line, column: 1 });
      if (pos) {
        const editorDom = editor.getDomNode();
        const rect = editorDom?.getBoundingClientRect();
        if (rect) {
          const glyphOffset = getGlyphMarginOffset(editor);
          openOrFocusPopup(newNote.id, rect.top + pos.top, rect.left + glyphOffset);
        }
      }
    }
  }, [enabled, tabId, addNote, editor]);
  addNoteAtLineRef.current = addNoteAtLine;

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

  // ── Gutter interactions: click, hover "+", scroll, context menu ────────
  // Depends only on [editor, enabled] — uses refs for notes data so
  // listeners aren't re-created on every note change.
  useEffect(() => {
    if (!editor || !enabled) return;

    let currentHoverLine: number | null = null;

    // Click on gutter glyph margin
    const mouseDownDisposable = editor.onMouseDown((e: editor.IEditorMouseEvent) => {
      if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN) {
        const line = e.target.position?.lineNumber;
        if (!line) return;

        const existingNote = notesRef.current.find((n) => n.line_number === line);
        if (existingNote) {
          // Open or focus popup for this note
          const pos = editor.getScrolledVisiblePosition({ lineNumber: line, column: 1 });
          if (pos) {
            const editorDom = editor.getDomNode();
            const rect = editorDom?.getBoundingClientRect();
            if (rect) {
              const glyphOffset = getGlyphMarginOffset(editor);
              openOrFocusPopup(existingNote.id, rect.top + pos.top, rect.left + glyphOffset);
            }
          }
        } else {
          // No note at this line — create one
          addNoteAtLineRef.current(line);
        }
      }
    });

    // Mouse move — show "+" decoration on lines without a note
    const mouseMoveDisposable = editor.onMouseMove((e: editor.IEditorMouseEvent) => {
      if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN) {
        const line = e.target.position?.lineNumber;
        if (line && line !== currentHoverLine) {
          currentHoverLine = line;
          const hasNote = notesRef.current.some((n) => n.line_number === line);
          if (!hasNote) {
            hoverDecorationRef.current = editor.deltaDecorations(hoverDecorationRef.current, [
              {
                range: new monaco.Range(line, 1, line, 1),
                options: {
                  isWholeLine: false,
                  glyphMarginClassName: 'larik-gutter-note-add-icon',
                  stickiness: monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges,
                },
              },
            ]);
          } else {
            // Line has a note — remove hover decoration
            if (hoverDecorationRef.current.length > 0) {
              hoverDecorationRef.current = editor.deltaDecorations(hoverDecorationRef.current, []);
            }
          }
        }
      }
    });

    // Mouse leave — clear hover decoration
    const mouseLeaveDisposable = editor.onMouseLeave(() => {
      currentHoverLine = null;
      if (hoverDecorationRef.current.length > 0) {
        hoverDecorationRef.current = editor.deltaDecorations(hoverDecorationRef.current, []);
      }
    });

    // Context menu (right-click) on gutter — also adds a note
    const contextMenuDisposable = editor.onContextMenu((e: editor.IEditorMouseEvent) => {
      if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN) {
        const line = e.target.position?.lineNumber;
        if (!line) return;
        const existingNote = notesRef.current.find((n) => n.line_number === line);
        if (!existingNote) {
          addNoteAtLineRef.current(line);
        }
      }
    });

    // Close unpinned popovers on scroll (pinned ones stay open)
    const scrollDisposable = editor.onDidScrollChange(() => {
      setActivePopups((current) => {
        const remaining: ActivePopup[] = [];
        for (const p of current) {
          const n = notesRef.current.find((nn) => nn.id === p.noteId);
          if (n?.pinned) remaining.push(p);
        }
        return remaining;
      });
    });

    disposablesRef.current.push(
      mouseDownDisposable,
      mouseMoveDisposable,
      mouseLeaveDisposable,
      contextMenuDisposable,
      scrollDisposable,
    );

    return () => {
      mouseDownDisposable.dispose();
      mouseMoveDisposable.dispose();
      mouseLeaveDisposable.dispose();
      contextMenuDisposable.dispose();
      scrollDisposable.dispose();
      // Clear hover decorations on cleanup
      if (hoverDecorationRef.current.length > 0) {
        hoverDecorationRef.current = editor.deltaDecorations(hoverDecorationRef.current, []);
      }
    };
  }, [editor, enabled]);

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

  // Close a single popover (called by NotePopover's backdrop / X button).
  const handleClosePopup = useCallback((noteId: string) => {
    setActivePopups((current) => current.filter((p) => p.noteId !== noteId));
  }, []);

  // Bring a popover to the front of the stack (most recently accessed on top).
  const handleFocusPopup = useCallback((noteId: string) => {
    setActivePopups((current) => {
      const z = ++nextZRef.current;
      return current.map((p) => (p.noteId === noteId ? { ...p, zIndex: z } : p));
    });
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      disposablesRef.current.forEach((d) => d.dispose());
      disposablesRef.current = [];
    };
  }, []);

  // Prune popovers whose underlying note has been deleted from the store.
  useEffect(() => {
    setActivePopups((current) => {
      const live = new Set(notes.map((n) => n.id));
      const filtered = current.filter((p) => live.has(p.noteId));
      return filtered.length === current.length ? current : filtered;
    });
  }, [notes]);

  // The active popup note objects (one NotePopover per open entry)
  const activeNotes = activePopups
    .map((p) => ({ popup: p, note: notes.find((n) => n.id === p.noteId) }))
    .filter((x) => x.note);

  // ── Portal element ──────────────────────────────────────────────────────
  // Render NotePopover via ReactDOM.createPortal into document.body so it
  // sits OUTSIDE Monaco's DOM tree. Monaco captures mouse events in its
  // container, which prevents clicks on buttons rendered inside the editor's
  // DOM hierarchy from reaching React handlers. By portaling to document.body,
  // the popover is in the top-level DOM and events flow normally.
  //
  // Multiple notes can be open simultaneously (pinned notes coexist; only one
  // unpinned note is open at a time). The most recently accessed popover is on
  // top via its zIndex (handled by handleFocusPopup bring-to-front).
  const gutterNotesPortal = activeNotes.length > 0
    ? createPortal(
        <>
          {activeNotes.map(({ popup, note }) => (
            <NotePopover
              key={note!.id}
              note={note!}
              position={{ top: popup.top, left: popup.left }}
              zIndex={popup.zIndex}
              theme={theme}
              onChange={handleNoteChange}
              onDelete={handleNoteDelete}
              onClose={handleClosePopup}
              onFocus={handleFocusPopup}
            />
          ))}
        </>,
        document.body,
      )
    : null;

  return { gutterNotesPortal, addNoteAtLine, notes };
}
