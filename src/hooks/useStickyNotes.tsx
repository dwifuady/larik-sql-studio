import { useState, useEffect, useCallback, useRef } from 'react';
import * as monaco from 'monaco-editor';
import {
    type StickyNote,
    type NoteColorKey,
    extractNotes,
    injectNoteIntoSql,
    serializeNote,
    moveNoteToLine,
} from '../utils/noteManager';
import { v4 as uuidv4 } from 'uuid';
import ReactDOM from 'react-dom';
import { StickyNote as StickyNoteComponent } from '../components/StickyNote';

// Custom event for CodeLens to trigger note creation
export const EVENT_ADD_STICKY_NOTE = 'larik:add-sticky-note';

interface UseStickyNotesProps {
    editor: monaco.editor.IStandaloneCodeEditor | null;
    model: monaco.editor.ITextModel | null;
    theme: 'dark' | 'light';
    onContentChange: (newContent: string) => void;
    enabled?: boolean;
}

export function useStickyNotes({ editor, model, theme, onContentChange, enabled = true }: UseStickyNotesProps) {
    const [notes, setNotes] = useState<StickyNote[]>([]);
    const viewZonesRef = useRef<Map<string, string>>(new Map()); // noteId -> viewZoneId
    const zoneHeightsRef = useRef<Map<string, number>>(new Map()); // noteId -> height
    const decoratorsRef = useRef<string[]>([]);
    const portalContainerRef = useRef<HTMLDivElement | null>(null);
    const [layoutVersion, setLayoutVersion] = useState(0);
    const dragStateRef = useRef<{ noteId: string; originalLine: number } | null>(null);

    // Create / cleanup portal container
    useEffect(() => {
        if (!portalContainerRef.current) {
            const div = document.createElement('div');
            div.id = 'sticky-notes-portal-container';
            div.style.position = 'absolute';
            div.style.top = '0';
            div.style.left = '0';
            div.style.width = '0';
            div.style.height = '0';
            div.style.pointerEvents = 'none';
            document.body.appendChild(div);
            portalContainerRef.current = div;
        }
        return () => {
            if (portalContainerRef.current && document.body.contains(portalContainerRef.current)) {
                document.body.removeChild(portalContainerRef.current);
                portalContainerRef.current = null;
            }
        };
    }, []);

    // Parse notes from content whenever model or content changes
    useEffect(() => {
        if (!editor || !model || !enabled) {
            setNotes([]);
            return;
        }

        const parseAndSetNotes = () => {
            const content = model.getValue();
            const extracted = extractNotes(content);
            setNotes(extracted);
        };

        parseAndSetNotes();

        const disposable = editor.onDidChangeModelContent(() => {
            parseAndSetNotes();
        });

        return () => {
            disposable.dispose();
        };
    }, [editor, model, enabled]);

    // ---- Note CRUD ----
    const handleNoteChange = useCallback((id: string, content: string, color: NoteColorKey, minimized: boolean) => {
        if (!model || !editor || !enabled) return;

        const currentContent = model.getValue();
        const currentNotes = extractNotes(currentContent);
        const targetNote = currentNotes.find(n => n.id === id);

        let newSql = currentContent;
        if (targetNote) {
            const lines = currentContent.split('\n');
            const lineIdx = targetNote.lineNumber - 1;
            lines[lineIdx] = serializeNote({ id, content, color, minimized });
            newSql = lines.join('\n');
        } else {
            const note: StickyNote = { id, content, color, minimized, lineNumber: 1 };
            newSql = injectNoteIntoSql(currentContent, note);
        }

        if (newSql !== currentContent) {
            onContentChange(newSql);
        }
        setNotes(prev => prev.map(n => n.id === id ? { ...n, content, color, minimized } : n));
    }, [editor, model, onContentChange, enabled]);

    const handleNoteDelete = useCallback((id: string) => {
        if (!model || !enabled) return;
        const currentContent = model.getValue();
        const newSql = currentContent
            .split('\n')
            .filter(line => !line.includes(id))
            .join('\n');
        onContentChange(newSql);
        setNotes(prev => prev.filter(n => n.id !== id));
    }, [model, onContentChange, enabled]);

    const handleCreateNote = useCallback((line: number) => {
        if (!model || !enabled) return;
        const newNoteId = uuidv4();
        const currentContent = model.getValue();
        const lines = currentContent.split('\n');
        const normalizedLine = Math.max(1, Math.min(line, lines.length + 1));
        const serialized = serializeNote({ id: newNoteId, content: '', color: 'yellow' as NoteColorKey });
        lines.splice(normalizedLine - 1, 0, serialized);
        onContentChange(lines.join('\n'));
    }, [model, onContentChange, enabled]);

    // ---- Drag to reposition ----
    const handleDragStart = useCallback((noteId: string) => {
        const note = notes.find(n => n.id === noteId);
        if (note) {
            dragStateRef.current = { noteId, originalLine: note.lineNumber };
        }
    }, [notes]);

    const handleDragMove = useCallback((deltaLines: number) => {
        if (!model || !enabled || !dragStateRef.current) return;
        const { noteId, originalLine } = dragStateRef.current;
        const targetLine = Math.max(1, originalLine + deltaLines);
        const currentContent = model.getValue();
        const newSql = moveNoteToLine(currentContent, noteId, targetLine);
        if (newSql !== currentContent) {
            // Update dragState originalLine to the new position for incremental moves
            const updatedNotes = extractNotes(newSql);
            const movedNote = updatedNotes.find(n => n.id === noteId);
            if (movedNote) {
                dragStateRef.current = { noteId, originalLine: movedNote.lineNumber };
            }
            onContentChange(newSql);
        }
    }, [model, onContentChange, enabled]);

    const handleDragEnd = useCallback(() => {
        dragStateRef.current = null;
    }, []);

    // Sync decorations + ViewZones
    useEffect(() => {
        if (!editor || !model) return;

        if (!enabled) {
            decoratorsRef.current = editor.deltaDecorations(decoratorsRef.current, []);
            editor.changeViewZones(changeAccessor => {
                viewZonesRef.current.forEach(vzId => changeAccessor.removeZone(vzId));
                viewZonesRef.current.clear();
                zoneHeightsRef.current.clear();
            });
            return;
        }

        // 1. Hide raw note comment lines
        const noteDecorations: monaco.editor.IModelDeltaDecoration[] = notes.map(note => ({
            range: new monaco.Range(note.lineNumber, 1, note.lineNumber, 1),
            options: {
                isWholeLine: true,
                className: 'hidden-monaco-line',
                inlineClassName: 'hidden-monaco-line',
            },
        }));
        decoratorsRef.current = editor.deltaDecorations(decoratorsRef.current, noteDecorations);

        // 2. Recreate ViewZones for all notes
        editor.changeViewZones(changeAccessor => {
            // Remove zones for notes that no longer exist
            const currentNoteIds = new Set(notes.map(n => n.id));
            viewZonesRef.current.forEach((vzId, noteId) => {
                if (!currentNoteIds.has(noteId)) {
                    changeAccessor.removeZone(vzId);
                    viewZonesRef.current.delete(noteId);
                    zoneHeightsRef.current.delete(noteId);
                }
            });

            // Create/update zones for current notes
            notes.forEach(note => {
                const existingZoneId = viewZonesRef.current.get(note.id);
                const height = zoneHeightsRef.current.get(note.id) ?? (note.minimized ? 28 : 80);

                if (existingZoneId) {
                    // Update afterLineNumber in case the note moved
                    changeAccessor.removeZone(existingZoneId);
                }

                const domNode = document.createElement('div');
                domNode.id = `note-zone-${note.id}`;
                domNode.style.width = '100%';
                domNode.style.height = `${height}px`;
                domNode.style.overflow = 'visible';
                domNode.style.zIndex = '10';

                const zoneId = changeAccessor.addZone({
                    afterLineNumber: note.lineNumber - 1,
                    heightInPx: height,
                    domNode: domNode,
                });
                viewZonesRef.current.set(note.id, zoneId);
            });

            setLayoutVersion(v => v + 1);
        });
    }, [editor, model, notes, enabled]);

    // Render Portals
    const StickyNotesRenderer = useCallback(() => {
        return (
            <>
                {notes.map(note => {
                    void layoutVersion;
                    const domNode = document.getElementById(`note-zone-${note.id}`);
                    if (!domNode) return null;

                    return ReactDOM.createPortal(
                        <div
                            className="w-full h-full pl-[52px] pr-4 py-1 pointer-events-auto"
                            onPointerDownCapture={() => {
                                document.querySelectorAll('[id^="note-zone-"]').forEach(el => {
                                    (el as HTMLElement).style.zIndex = '10';
                                });
                                const zoneEl = document.getElementById(`note-zone-${note.id}`);
                                if (zoneEl) zoneEl.style.zIndex = '100';
                            }}
                        >
                            <StickyNoteComponent
                                key={note.id}
                                id={note.id}
                                initialContent={note.content}
                                initialColor={note.color}
                                initialMinimized={note.minimized}
                                theme={theme}
                                onChange={handleNoteChange}
                                onDelete={handleNoteDelete}
                                onResize={(height) => {
                                    zoneHeightsRef.current.set(note.id, height);
                                    if (!editor) return;
                                    const zoneId = viewZonesRef.current.get(note.id);
                                    if (zoneId) {
                                        const node = document.getElementById(`note-zone-${note.id}`);
                                        if (node) {
                                            node.style.height = `${height}px`;
                                        }
                                        editor.changeViewZones(accessor => {
                                            accessor.layoutZone(zoneId);
                                        });
                                    }
                                }}
                                onDragStart={handleDragStart}
                                onDragMove={handleDragMove}
                                onDragEnd={handleDragEnd}
                            />
                        </div>,
                        domNode
                    );
                })}
            </>
        );
    }, [notes, layoutVersion, theme, handleNoteChange, handleNoteDelete, handleDragStart, handleDragMove, handleDragEnd, editor]);

    // Listen for "Add Note" events
    useEffect(() => {
        if (!enabled) return;
        const handleAddNote = (e: any) => {
            const { line } = e.detail || {};
            if (line) handleCreateNote(line);
        };
        window.addEventListener(EVENT_ADD_STICKY_NOTE, handleAddNote);
        return () => window.removeEventListener(EVENT_ADD_STICKY_NOTE, handleAddNote);
    }, [handleCreateNote, enabled]);

    return { StickyNotesRenderer };
}
