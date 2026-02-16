import { StateCreator } from 'zustand';
import type { Snippet, CreateSnippetInput, UpdateSnippetInput } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

export interface SnippetsSlice {
    snippets: Snippet[];
    snippetsLoading: boolean;

    loadSnippets: () => Promise<void>;
    createSnippet: (input: CreateSnippetInput) => Promise<Snippet | null>;
    updateSnippet: (id: string, input: UpdateSnippetInput) => Promise<void>;
    deleteSnippet: (id: string) => Promise<boolean>;
    resetBuiltinSnippet: (id: string) => Promise<void>;
    importSnippets: (snippets: CreateSnippetInput[]) => Promise<number>;
    getEnabledSnippets: () => Snippet[];
}

export const createSnippetsSlice: StateCreator<AppState, [], [], SnippetsSlice> = (set, get) => ({
    snippets: [],
    snippetsLoading: false,

    loadSnippets: async () => {
        set({ snippetsLoading: true });
        try {
            const snippets = await api.getSnippets();
            set({ snippets, snippetsLoading: false });
        } catch (error) {
            console.error('Failed to load snippets:', error);
            set({ snippetsLoading: false });
        }
    },

    createSnippet: async (input) => {
        try {
            const snippet = await api.createSnippet(input);
            set((state) => ({ snippets: [...state.snippets, snippet] }));
            return snippet;
        } catch (error) {
            console.error('Failed to create snippet:', error);
            return null;
        }
    },

    updateSnippet: async (id, input) => {
        try {
            const updated = await api.updateSnippet(id, input);
            if (updated) {
                set((state) => ({
                    snippets: state.snippets.map((s) => (s.id === id ? updated : s)),
                }));
            }
        } catch (error) {
            console.error('Failed to update snippet:', error);
        }
    },

    deleteSnippet: async (id) => {
        try {
            const success = await api.deleteSnippet(id);
            if (success) {
                set((state) => ({
                    snippets: state.snippets.filter((s) => s.id !== id),
                }));
            }
            return success;
        } catch (error) {
            console.error('Failed to delete snippet:', error);
            return false;
        }
    },

    resetBuiltinSnippet: async (id) => {
        try {
            const reset = await api.resetBuiltinSnippet(id);
            if (reset) {
                set((state) => ({
                    snippets: state.snippets.map((s) => (s.id === id ? reset : s)),
                }));
            }
        } catch (error) {
            console.error('Failed to reset snippet:', error);
        }
    },

    importSnippets: async (snippets) => {
        try {
            const count = await api.importSnippets(snippets);
            await get().loadSnippets(); // Reload to get new IDs
            return count;
        } catch (error) {
            console.error('Failed to import snippets:', error);
            return 0;
        }
    },

    getEnabledSnippets: () => {
        return get().snippets.filter((s) => s.enabled);
    }
});
