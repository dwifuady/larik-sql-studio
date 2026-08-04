// Sticky Notes API — Tauri invoke wrappers for the gutter-icon note system.
import { invoke } from '@tauri-apps/api/core';

export interface StickyNote {
  id: string;
  tab_id: string;
  line_number: number;
  content: string;
  color: string;
  minimized: boolean;
  /** Persisted popover width in pixels (null/undefined = use default). */
  width?: number | null;
  /** When true the popover stays open on outside click. */
  pinned?: boolean;
  created_at: string;
  updated_at: string;
}

export const getTabNotes = (tabId: string) =>
  invoke<StickyNote[]>('get_tab_notes', { tabId });

export const saveNote = (note: StickyNote) =>
  invoke<void>('save_note', { note });

export const deleteNote = (noteId: string, tabId: string) =>
  invoke<void>('delete_note', { noteId, tabId });

export const moveNote = (noteId: string, tabId: string, newLine: number) =>
  invoke<void>('move_note', { noteId, tabId, newLine });

export const clearTabNotes = (tabId: string) =>
  invoke<void>('clear_tab_notes', { tabId });
