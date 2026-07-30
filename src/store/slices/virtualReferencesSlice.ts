import { StateCreator } from 'zustand';
import type { CreateVirtualReferenceInput, VirtualReference } from '../../types';
import * as api from '../../api';
import type { AppState } from '../index';

/** The column a user-defined reference is being defined for. */
export interface ReferenceEditorSource {
    schema: string;
    table: string;
    column: string;
}

export interface ReferenceEditorState {
    open: boolean;
    source: ReferenceEditorSource | null;
    /** Set when editing an existing user-defined reference. */
    existingId: string | null;
    /** Current target of the reference being edited, for prefilling. */
    existingTarget: { schema: string; table: string; column: string } | null;
    saving: boolean;
}

export interface VirtualReferencesSlice {
    /** User-defined references for the active space + database. */
    virtualReferences: VirtualReference[];
    virtualReferencesLoading: boolean;
    referenceEditor: ReferenceEditorState;

    loadVirtualReferences: (database?: string | null) => Promise<void>;
    saveVirtualReference: (input: CreateVirtualReferenceInput) => Promise<VirtualReference | null>;
    removeVirtualReference: (id: string) => Promise<void>;

    openReferenceEditor: (
        source: ReferenceEditorSource,
        existing?: { id: string; schema: string; table: string; column: string } | null
    ) => void;
    closeReferenceEditor: () => void;
}

const initialEditor: ReferenceEditorState = {
    open: false,
    source: null,
    existingId: null,
    existingTarget: null,
    saving: false,
};

export const createVirtualReferencesSlice: StateCreator<AppState, [], [], VirtualReferencesSlice> = (set, get) => ({
    virtualReferences: [],
    virtualReferencesLoading: false,
    referenceEditor: initialEditor,

    loadVirtualReferences: async (database) => {
        const spaceId = get().activeSpaceId;
        const targetDb =
            database ??
            get().schemaInfo?.database_name ??
            get().getActiveTab()?.database ??
            get().getActiveSpace()?.connection_database;

        if (!spaceId || !targetDb) {
            set({ virtualReferences: [] });
            return;
        }

        set({ virtualReferencesLoading: true });
        try {
            const references = await api.getVirtualReferences(spaceId, targetDb);
            set({ virtualReferences: references, virtualReferencesLoading: false });
        } catch (error) {
            console.error('Failed to load virtual references:', error);
            set({ virtualReferences: [], virtualReferencesLoading: false });
        }
    },

    saveVirtualReference: async (input) => {
        const spaceId = get().activeSpaceId;
        const targetDb = get().schemaInfo?.database_name ?? get().getActiveTab()?.database ?? null;

        if (!spaceId || !targetDb) {
            get().addToast({ type: 'error', message: 'No active database to attach the reference to' });
            return null;
        }

        set((state) => ({ referenceEditor: { ...state.referenceEditor, saving: true } }));
        try {
            const saved = await api.saveVirtualReference(spaceId, targetDb, input);
            set((state) => ({
                // Replace any existing entry for the same source column.
                virtualReferences: [
                    ...state.virtualReferences.filter(
                        (reference) =>
                            !(
                                reference.source_schema.toLowerCase() === saved.source_schema.toLowerCase() &&
                                reference.source_table.toLowerCase() === saved.source_table.toLowerCase() &&
                                reference.source_column.toLowerCase() === saved.source_column.toLowerCase()
                            )
                    ),
                    saved,
                ],
                referenceEditor: initialEditor,
            }));
            // Any open preview was built from the previous target — drop it so
            // the next Shift+Space resolves against the new reference.
            get().clearReferencePreview();
            get().addToast({
                type: 'success',
                message: `${saved.source_column} now references ${saved.target_schema}.${saved.target_table}`,
            });
            return saved;
        } catch (error) {
            console.error('Failed to save virtual reference:', error);
            set((state) => ({ referenceEditor: { ...state.referenceEditor, saving: false } }));
            get().addToast({
                type: 'error',
                message: error instanceof Error ? error.message : 'Failed to save reference',
            });
            return null;
        }
    },

    removeVirtualReference: async (id) => {
        try {
            await api.deleteVirtualReference(id);
            set((state) => ({
                virtualReferences: state.virtualReferences.filter((reference) => reference.id !== id),
            }));
            // The open preview was built from this reference — drop it.
            get().clearReferencePreview();
            get().addToast({ type: 'success', message: 'Custom reference removed' });
        } catch (error) {
            console.error('Failed to delete virtual reference:', error);
            get().addToast({
                type: 'error',
                message: error instanceof Error ? error.message : 'Failed to remove reference',
            });
        }
    },

    openReferenceEditor: (source, existing) => {
        set({
            referenceEditor: {
                open: true,
                source,
                existingId: existing?.id ?? null,
                existingTarget: existing
                    ? { schema: existing.schema, table: existing.table, column: existing.column }
                    : null,
                saving: false,
            },
        });
    },

    closeReferenceEditor: () => set({ referenceEditor: initialEditor }),
});
