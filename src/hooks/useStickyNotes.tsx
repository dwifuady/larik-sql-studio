import { useState, useEffect, useCallback, useRef } from 'react';
import * as monaco from 'monaco-editor';
import { StickyNote, extractNotes, injectNoteIntoSql, serializeNote } from '../utils/noteManager';

import { v4 as uuidv4 } from 'uuid';
import ReactDOM from 'react-dom';

import { StickyNote as StickyNoteComponent } from '../components/StickyNote';

// Custom event for CodeLens to trigger note creation
export const EVENT_ADD_STICKY_NOTE = 'larik:add-sticky-note';

interface UseStickyNotesProps {
    editor: monaco.editor.IStandaloneCodeEditor | null;
    model: monaco.editor.ITextModel | null;
    onContentChange: (newContent: string) => void;
    enabled?: boolean;
}

export function useStickyNotes({ editor, model, onContentChange, enabled = true }: UseStickyNotesProps) {
    const [notes, setNotes] = useState<StickyNote[]>([]);
    const viewZonesRef = useRef<Map<string, string>>(new Map()); // noteId -> viewZoneId
    const decoratorsRef = useRef<string[]>([]);
    const portalContainerRef = useRef<HTMLDivElement | null>(null);
    const [layoutVersion, setLayoutVersion] = useState(0); // Force re-render when ViewZones are updated

    // Create a container for React portals if it doesn't exist
    useEffect(() => {
        if (!portalContainerRef.current) {
            const div = document.createElement('div');
            div.id = 'sticky-notes-portal-container';
            div.style.position = 'absolute';
            div.style.top = '0';
            div.style.left = '0';
            div.style.width = '0';
            div.style.height = '0';
            div.style.pointerEvents = 'none'; // Users interact with the portal content, not the container
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

        // Initial parse
        parseAndSetNotes();

        // Listen for content changes (includes tab switching via setValue)
        const disposable = editor.onDidChangeModelContent(() => {
            parseAndSetNotes();
        });

        return () => {
            disposable.dispose();
        };
    }, [editor, model, enabled]);

    // Handle Note Changes (from UI)
    const handleNoteChange = useCallback((id: string, content: string, color: string, minimized: boolean) => {
        if (!model || !editor || !enabled) return;

        const currentContent = model.getValue();
        const note = notes.find(n => n.id === id);

        if (note) {
            // Create updated note object
            const updatedNote = { ...note, content, color, minimized };

            // Inject into SQL (this handles both update and insert logic in the utils)
            // but `injectNoteIntoSql` might rely on line numbers which change.
            // Ideally we replace the EXACT line corresponding to this note.

            // Since we know the ID, we can find the line in the current text that has this ID.
            // extractNotes returns line numbers.
            // Let's re-extract to be sure of current line positions
            const currentNotes = extractNotes(currentContent);
            const targetNote = currentNotes.find(n => n.id === id);

            let newSql = currentContent;
            if (targetNote) {
                // Update existing line
                const lines = currentContent.split('\n');
                const lineIdx = targetNote.lineNumber - 1; // 0-based
                lines[lineIdx] = serializeNote({ id, content, color, minimized });
                newSql = lines.join('\n');
            } else {
                // Should not happen if UI is visible, but maybe it was deleted physically?
                // Do nothing or re-insert?
                // Re-insert is safer.
                // fallback to note.lineNumber
                newSql = injectNoteIntoSql(currentContent, updatedNote);
            }

            // Apply edit to model
            // We use pushEditOperations to preserve undo stack
            // Replacing the whole text is heavy but easiest for now.
            // Optimization: Calculate range for the specific line.
            if (newSql !== currentContent) {
                // Disable our own listeners to avoid loops?
                // editor.executeEdits('sticky-notes', [{
                //     range: model.getFullModelRange(),
                //     text: newSql
                // }]);
                // Better: let the parent handle it via onContentChange if provided, 
                // OR update model directly.
                onContentChange(newSql);
            }

            // Update local state is automatic via re-render/re-parse cycle or optimistic?
            // Optimistic update for responsiveness
            setNotes(prev => prev.map(n => n.id === id ? updatedNote : n));
        }
    }, [editor, model, notes, onContentChange, enabled]);

    const handleNoteDelete = useCallback((id: string) => {
        if (!model || !enabled) return;

        const currentContent = model.getValue();
        const lines = currentContent.split('\n');
        // Filter out the line containing this note ID
        const newLines = lines.filter(line => !line.includes(id));
        const newSql = newLines.join('\n');

        onContentChange(newSql);
        setNotes(prev => prev.filter(n => n.id !== id));
    }, [model, onContentChange]);

    const handleCreateNote = useCallback((line: number) => {
        if (!model || !enabled) return;

        const newNote: StickyNote = {
            id: uuidv4(),
            content: '',
            color: '#fff9c4',
            minimized: false,
            lineNumber: line
        };

        const currentContent = model.getValue();
        // Inject at the specific line
        const lines = currentContent.split('\n');
        // Insert at index (line - 1)
        const normalizedLine = Math.max(1, Math.min(line, lines.length + 1));
        const serialized = serializeNote({ id: newNote.id, content: '', color: newNote.color });
        lines.splice(normalizedLine - 1, 0, serialized);
        const newSql = lines.join('\n');

        onContentChange(newSql);
        // State update will happen on next render/effect
    }, [model, onContentChange]);

    // Sync Decorations (hide note text) and ViewZones (show note UI)
    useEffect(() => {
        if (!editor || !model) return;

        if (!enabled) {
            // Cleanup decorations
            decoratorsRef.current = editor.deltaDecorations(decoratorsRef.current, []);
            // Cleanup zones
            editor.changeViewZones(changeAccessor => {
                viewZonesRef.current.forEach((vzId) => {
                    changeAccessor.removeZone(vzId);
                });
                viewZonesRef.current.clear();
            });
            return;
        }

        // 1. Hide the raw note comment text using decorations
        const noteDecorations: monaco.editor.IModelDeltaDecoration[] = notes.map(note => ({
            range: new monaco.Range(note.lineNumber, 1, note.lineNumber, 1),
            options: {
                isWholeLine: true,
                className: 'hidden-monaco-line',
                inlineClassName: 'hidden-monaco-line',
            }
        }));
        decoratorsRef.current = editor.deltaDecorations(decoratorsRef.current, noteDecorations);

        // 2. Recreate ALL ViewZones every time notes change
        editor.changeViewZones(changeAccessor => {
            // Remove all existing zones
            viewZonesRef.current.forEach((vzId) => {
                changeAccessor.removeZone(vzId);
            });
            viewZonesRef.current.clear();

            // Create fresh zones for all notes
            notes.forEach(note => {
                const domNode = document.createElement('div');
                domNode.id = `note-zone-${note.id}`;
                domNode.style.zIndex = '10';
                domNode.style.width = '100%';

                const zoneId = changeAccessor.addZone({
                    afterLineNumber: note.lineNumber - 1,
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
                {
                    notes.map(note => {
                        void layoutVersion;
                        const domNode = document.getElementById(`note-zone-${note.id}`);
                        if (!domNode) return null;

                        return ReactDOM.createPortal(
                            <div
                                className="w-full h-full pl-[50px] pr-4 py-1 pointer-events-auto"
                                onMouseDownCapture={() => {
                                    // Bring this note to front by bumping z-index
                                    document.querySelectorAll('[id^="note-zone-"]').forEach(el => {
                                        (el as HTMLElement).style.zIndex = '10';
                                    });
                                    const zoneEl = document.getElementById(`note-zone-${note.id}`);
                                    if (zoneEl) zoneEl.style.zIndex = '100';
                                }}
                            >
                                <StickyNoteComponent
                                    id={note.id}
                                    initialContent={note.content}
                                    initialColor={note.color}
                                    initialMinimized={note.minimized}
                                    onChange={handleNoteChange}
                                    onDelete={handleNoteDelete}
                                    onResize={() => {
                                        // Trigger re-layout
                                        if (!editor) return;
                                        const zoneId = viewZonesRef.current.get(note.id);
                                        if (zoneId) {
                                            editor.changeViewZones(accessor => {
                                                accessor.layoutZone(zoneId);
                                            });
                                        }
                                    }}
                                />
                            </div>,
                            domNode
                        );
                    })}
            </>
        );
    }, [notes, layoutVersion, handleNoteChange, handleNoteDelete]);

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

    return {
        StickyNotesRenderer
    };
}
