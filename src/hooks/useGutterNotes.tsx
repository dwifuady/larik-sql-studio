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
  const [activePopup, setActivePopup] = useState<ActivePopup | null>(null);
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
          setActivePopup({
            noteId: newNote.id,
            top: rect.top + pos.top,
            left: rect.left + glyphOffset,
          });
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
          // Toggle popup for this note
          const pos = editor.getScrolledVisiblePosition({ lineNumber: line, column: 1 });
          if (pos) {
            const editorDom = editor.getDomNode();
            const rect = editorDom?.getBoundingClientRect();
            if (rect) {
              const glyphOffset = getGlyphMarginOffset(editor);
              setActivePopup({
                noteId: existingNote.id,
                top: rect.top + pos.top,
                left: rect.left + glyphOffset,
              });
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

    // Close popup on scroll
    const scrollDisposable = editor.onDidScrollChange(() => {
      setActivePopup(null);
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

  // Close popup handler
  const handleClosePopup = useCallback(() => {
    setActivePopup(null);
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      disposablesRef.current.forEach((d) => d.dispose());
      disposablesRef.current = [];
    };
  }, []);

  // The active popup note object
  const activeNote = activePopup ? notes.find((n) => n.id === activePopup.noteId) : null;

  // ── Portal element ──────────────────────────────────────────────────────
  // Render NotePopover via ReactDOM.createPortal into document.body so it
  // sits OUTSIDE Monaco's DOM tree. Monaco captures mouse events in its
  // container, which prevents clicks on buttons rendered inside the editor's
  // DOM hierarchy from reaching React handlers. By portaling to document.body,
  // the popover is in the top-level DOM and events flow normally.
  //
  // IMPORTANT: this must be returned as a plain React *element*, NOT a
  // memoized component rendered as `<GutterNotesRenderer />`. A useCallback
  // component changes identity whenever its deps change, and rendering a
  // changed component *type* makes React unmount + remount the whole subtree
  // — which tore down NotePopover on every store update (the "blink" bug that
  // made buttons feel unclickable). Returning an element lets React reconcile
  // NotePopover by position and merely update its props.
  const gutterNotesPortal =
    activePopup && activeNote
      ? createPortal(
          <NotePopover
            key={activeNote.id}
            note={activeNote}
            position={{ top: activePopup.top, left: activePopup.left }}
            theme={theme}
            onChange={handleNoteChange}
            onDelete={handleNoteDelete}
            onClose={handleClosePopup}
          />,
          document.body,
        )
      : null;

  return { gutterNotesPortal, addNoteAtLine, notes };
}
