import { create } from 'zustand';

// Import Slices
import { createSpacesSlice, SpacesSlice } from './slices/spacesSlice';
import { createTabsSlice, TabsSlice } from './slices/tabsSlice';
import { createFoldersSlice, FoldersSlice } from './slices/foldersSlice';
import { createSchemaSlice, SchemaSlice } from './slices/schemaSlice';
import { createSnippetsSlice, SnippetsSlice } from './slices/snippetsSlice';
import { createArchiveSlice, ArchiveSlice } from './slices/archiveSlice';
import { createUISlice, UISlice } from './slices/uiSlice';
import { createQueriesSlice, QueriesSlice } from './slices/queriesSlice';

// Combine all slice interfaces into the main AppState
export type AppState =
  & SpacesSlice
  & TabsSlice
  & FoldersSlice
  & SchemaSlice
  & SnippetsSlice
  & ArchiveSlice
  & UISlice
  & QueriesSlice;

// Create the combined store
export const useAppStore = create<AppState>((...a) => ({
  ...createSpacesSlice(...a),
  ...createTabsSlice(...a),
  ...createFoldersSlice(...a),
  ...createSchemaSlice(...a),
  ...createSnippetsSlice(...a),
  ...createArchiveSlice(...a),
  ...createUISlice(...a),
  ...createQueriesSlice(...a),
}));
