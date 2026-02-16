// Snippets Management Dialog (T046)
// Allows users to view, create, edit, delete snippets and import from DBeaver

import { useState, useEffect, useCallback } from 'react';
import { useAppStore } from '../store';
import type { Snippet } from '../types';
import { importDbeaverTemplates } from '../utils/dbeaverImport';

interface SnippetsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SnippetsDialog({ isOpen, onClose }: SnippetsDialogProps) {
  const {
    snippets,
    loadSnippets,
    createSnippet,
    updateSnippet,
    deleteSnippet,
    resetBuiltinSnippet,
    importSnippets,
    addToast,
  } = useAppStore();

  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [editingSnippet, setEditingSnippet] = useState<Snippet | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [importLoading, setImportLoading] = useState(false);

  // Form state for creating/editing
  const [formTrigger, setFormTrigger] = useState('');
  const [formName, setFormName] = useState('');
  const [formContent, setFormContent] = useState('');
  const [formDescription, setFormDescription] = useState('');
  const [formCategory, setFormCategory] = useState('');

  // Load snippets when dialog opens
  useEffect(() => {
    if (isOpen) {
      loadSnippets();
    }
  }, [isOpen, loadSnippets]);

  // Get unique categories
  const categories = Array.from(
    new Set(snippets.map(s => s.category).filter(Boolean))
  ).sort() as string[];

  // Filter snippets
  const filteredSnippets = snippets.filter(snippet => {
    const matchesSearch = 
      searchQuery === '' ||
      snippet.trigger.toLowerCase().includes(searchQuery.toLowerCase()) ||
      snippet.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      snippet.content.toLowerCase().includes(searchQuery.toLowerCase());
    
    const matchesCategory = 
      selectedCategory === null || 
      snippet.category === selectedCategory;

    return matchesSearch && matchesCategory;
  });

  // Reset form
  const resetForm = useCallback(() => {
    setFormTrigger('');
    setFormName('');
    setFormContent('');
    setFormDescription('');
    setFormCategory('');
    setEditingSnippet(null);
    setIsCreating(false);
  }, []);

  // Start creating new snippet
  const handleStartCreate = () => {
    resetForm();
    setIsCreating(true);
  };

  // Start editing snippet
  const handleStartEdit = (snippet: Snippet) => {
    setFormTrigger(snippet.trigger);
    setFormName(snippet.name);
    setFormContent(snippet.content);
    setFormDescription(snippet.description || '');
    setFormCategory(snippet.category || '');
    setEditingSnippet(snippet);
    setIsCreating(false);
  };

  // Save snippet (create or update)
  const handleSave = async () => {
    if (!formTrigger.trim() || !formName.trim() || !formContent.trim()) {
      addToast({
        type: 'error',
        message: 'Validation Error: Trigger, name, and content are required',
      });
      return;
    }

    try {
      if (isCreating) {
        await createSnippet({
          trigger: formTrigger.trim(),
          name: formName.trim(),
          content: formContent,
          description: formDescription.trim() || null,
          category: formCategory.trim() || null,
        });
        addToast({
          type: 'success',
          message: `Snippet "${formName}" has been created`,
        });
      } else if (editingSnippet) {
        await updateSnippet(editingSnippet.id, {
          trigger: formTrigger.trim(),
          name: formName.trim(),
          content: formContent,
          description: formDescription.trim() || null,
          category: formCategory.trim() || null,
        });
        addToast({
          type: 'success',
          message: `Snippet "${formName}" has been updated`,
        });
      }
      resetForm();
    } catch (error) {
      addToast({
        type: 'error',
        message: error instanceof Error ? error.message : 'Failed to save snippet',
      });
    }
  };

  // Toggle snippet enabled/disabled
  const handleToggleEnabled = async (snippet: Snippet) => {
    await updateSnippet(snippet.id, { enabled: !snippet.enabled });
  };

  // Delete snippet
  const handleDelete = async (snippet: Snippet) => {
    if (snippet.is_builtin) {
      addToast({
        type: 'error',
        message: 'Built-in snippets cannot be deleted. You can disable them instead.',
      });
      return;
    }

    if (confirm(`Delete snippet "${snippet.name}"?`)) {
      const deleted = await deleteSnippet(snippet.id);
      if (deleted) {
        addToast({
          type: 'success',
          message: `Snippet "${snippet.name}" has been deleted`,
        });
        if (editingSnippet?.id === snippet.id) {
          resetForm();
        }
      }
    }
  };

  // Reset builtin snippet
  const handleReset = async (snippet: Snippet) => {
    if (!snippet.is_builtin) return;

    if (confirm(`Reset "${snippet.name}" to default?`)) {
      await resetBuiltinSnippet(snippet.id);
      addToast({
        type: 'success',
        message: `Snippet "${snippet.name}" has been reset to default`,
      });
      if (editingSnippet?.id === snippet.id) {
        resetForm();
      }
    }
  };

  // Handle file input for DBeaver import
  const handleFileInput = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;

    try {
      setImportLoading(true);
      const xmlContent = await file.text();
      
      // Parse and convert snippets
      const newSnippets = importDbeaverTemplates(xmlContent);
      
      if (newSnippets.length === 0) {
        addToast({
          type: 'warning',
          message: 'No valid SQL snippets were found in the file',
        });
        setImportLoading(false);
        return;
      }

      // Import snippets
      const count = await importSnippets(newSnippets);
      
      addToast({
        type: 'success',
        message: `Imported ${count} new snippets from DBeaver templates`,
      });
    } catch (error) {
      console.error('Import error:', error);
      addToast({
        type: 'error',
        message: error instanceof Error ? error.message : 'Failed to import DBeaver templates',
      });
    } finally {
      setImportLoading(false);
      // Reset file input
      event.target.value = '';
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div 
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Dialog */}
      <div className="relative w-[1000px] max-w-[95vw] h-[700px] max-h-[90vh] bg-[var(--bg-primary)] rounded-xl shadow-2xl border border-white/10 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-indigo-500/20 flex items-center justify-center">
              <svg className="w-5 h-5 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
              </svg>
            </div>
            <div>
              <h2 className="text-lg font-semibold text-[var(--text-primary)]">SQL Snippets</h2>
              <p className="text-sm text-[var(--text-muted)]">
                {snippets.length} snippets • Type trigger text and press Tab to expand
              </p>
            </div>
          </div>
          
          <button
            onClick={onClose}
            className="p-2 rounded-lg text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-white/5 transition-colors"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="flex flex-1 min-h-0">
          {/* Snippet List */}
          <div className="w-[400px] border-r border-white/10 flex flex-col">
            {/* Search and filters */}
            <div className="p-4 border-b border-white/5 space-y-3">
              <div className="relative">
                <svg className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-[var(--text-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                </svg>
                <input
                  type="text"
                  placeholder="Search snippets..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="w-full pl-10 pr-4 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                />
              </div>
              
              <div className="flex items-center gap-2 overflow-x-auto pb-1">
                <button
                  onClick={() => setSelectedCategory(null)}
                  className={`px-3 py-1 rounded-full text-xs font-medium whitespace-nowrap transition-colors ${
                    selectedCategory === null
                      ? 'bg-indigo-500 text-white'
                      : 'bg-white/5 text-[var(--text-muted)] hover:bg-white/10'
                  }`}
                >
                  All
                </button>
                {categories.map(cat => (
                  <button
                    key={cat}
                    onClick={() => setSelectedCategory(cat)}
                    className={`px-3 py-1 rounded-full text-xs font-medium whitespace-nowrap transition-colors ${
                      selectedCategory === cat
                        ? 'bg-indigo-500 text-white'
                        : 'bg-white/5 text-[var(--text-muted)] hover:bg-white/10'
                    }`}
                  >
                    {cat}
                  </button>
                ))}
              </div>
            </div>

            {/* Snippet list */}
            <div className="flex-1 overflow-y-auto">
              {filteredSnippets.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-full text-[var(--text-muted)]">
                  <svg className="w-12 h-12 mb-3 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                  </svg>
                  <p className="text-sm">No snippets found</p>
                </div>
              ) : (
                filteredSnippets.map(snippet => (
                  <div
                    key={snippet.id}
                    onClick={() => handleStartEdit(snippet)}
                    className={`px-4 py-3 border-b border-white/5 cursor-pointer transition-colors ${
                      editingSnippet?.id === snippet.id
                        ? 'bg-indigo-500/10'
                        : 'hover:bg-white/5'
                    }`}
                  >
                    <div className="flex items-center justify-between mb-1">
                      <div className="flex items-center gap-2">
                        <code className="px-2 py-0.5 bg-white/10 rounded text-xs text-indigo-300 font-mono">
                          {snippet.trigger}
                        </code>
                        {snippet.is_builtin && (
                          <span className="px-1.5 py-0.5 bg-amber-500/20 rounded text-[10px] text-amber-400 uppercase">
                            Built-in
                          </span>
                        )}
                        {!snippet.enabled && (
                          <span className="px-1.5 py-0.5 bg-red-500/20 rounded text-[10px] text-red-400 uppercase">
                            Disabled
                          </span>
                        )}
                      </div>
                      {snippet.category && (
                        <span className="text-xs text-[var(--text-muted)]">{snippet.category}</span>
                      )}
                    </div>
                    <p className={`text-sm font-medium ${snippet.enabled ? 'text-[var(--text-primary)]' : 'text-[var(--text-muted)]'}`}>
                      {snippet.name}
                    </p>
                    <p className="text-xs text-[var(--text-muted)] truncate mt-1 font-mono">
                      {snippet.content.split('\n')[0]}
                    </p>
                  </div>
                ))
              )}
            </div>

            {/* Actions */}
            <div className="p-4 border-t border-white/10 space-y-2">
              <button
                onClick={handleStartCreate}
                className="w-full px-4 py-2 bg-indigo-500 hover:bg-indigo-600 text-white rounded-lg text-sm font-medium transition-colors flex items-center justify-center gap-2"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
                New Snippet
              </button>
              
              <label className={`w-full px-4 py-2 bg-white/5 hover:bg-white/10 text-[var(--text-primary)] rounded-lg text-sm font-medium transition-colors flex items-center justify-center gap-2 cursor-pointer ${importLoading ? 'opacity-50 pointer-events-none' : ''}`}>
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                </svg>
                {importLoading ? 'Importing...' : 'Import from DBeaver'}
                <input
                  type="file"
                  accept=".xml"
                  onChange={handleFileInput}
                  className="hidden"
                  disabled={importLoading}
                />
              </label>
            </div>
          </div>

          {/* Editor Panel */}
          <div className="flex-1 flex flex-col">
            {(editingSnippet || isCreating) ? (
              <>
                <div className="p-4 border-b border-white/10">
                  <h3 className="text-sm font-medium text-[var(--text-primary)] mb-1">
                    {isCreating ? 'Create New Snippet' : 'Edit Snippet'}
                  </h3>
                  <p className="text-xs text-[var(--text-muted)]">
                    {isCreating 
                      ? 'Define a new SQL snippet with a trigger text'
                      : editingSnippet?.is_builtin 
                        ? 'Editing built-in snippet (cannot be deleted)'
                        : 'Edit your custom snippet'
                    }
                  </p>
                </div>

                <div className="flex-1 overflow-y-auto p-4 space-y-4">
                  {/* Trigger & Name */}
                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <label className="block text-xs font-medium text-[var(--text-muted)] mb-1.5">
                        Trigger <span className="text-red-400">*</span>
                      </label>
                      <input
                        type="text"
                        value={formTrigger}
                        onChange={(e) => setFormTrigger(e.target.value)}
                        placeholder="sel, ins, upd..."
                        className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50 font-mono"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--text-muted)] mb-1.5">
                        Name <span className="text-red-400">*</span>
                      </label>
                      <input
                        type="text"
                        value={formName}
                        onChange={(e) => setFormName(e.target.value)}
                        placeholder="SELECT statement"
                        className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                      />
                    </div>
                  </div>

                  {/* Category */}
                  <div>
                    <label className="block text-xs font-medium text-[var(--text-muted)] mb-1.5">
                      Category
                    </label>
                    <input
                      type="text"
                      value={formCategory}
                      onChange={(e) => setFormCategory(e.target.value)}
                      placeholder="Select, Insert, DDL..."
                      list="category-suggestions"
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                    />
                    <datalist id="category-suggestions">
                      {categories.map(cat => (
                        <option key={cat} value={cat} />
                      ))}
                    </datalist>
                  </div>

                  {/* Description */}
                  <div>
                    <label className="block text-xs font-medium text-[var(--text-muted)] mb-1.5">
                      Description
                    </label>
                    <input
                      type="text"
                      value={formDescription}
                      onChange={(e) => setFormDescription(e.target.value)}
                      placeholder="Brief description of what this snippet does"
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                    />
                  </div>

                  {/* Content */}
                  <div className="flex-1">
                    <label className="block text-xs font-medium text-[var(--text-muted)] mb-1.5">
                      Content <span className="text-red-400">*</span>
                      <span className="ml-2 font-normal text-[var(--text-muted)]">
                        Use $&#123;cursor&#125; for cursor position, $&#123;1:placeholder&#125; for tab stops
                      </span>
                    </label>
                    <textarea
                      value={formContent}
                      onChange={(e) => setFormContent(e.target.value)}
                      placeholder="SELECT * FROM ${cursor}"
                      rows={12}
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-[var(--text-primary)] placeholder-[var(--text-muted)] focus:outline-none focus:ring-2 focus:ring-indigo-500/50 font-mono resize-none"
                    />
                  </div>
                </div>

                {/* Actions */}
                <div className="p-4 border-t border-white/10 flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    {editingSnippet && (
                      <>
                        <button
                          onClick={() => handleToggleEnabled(editingSnippet)}
                          className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-colors ${
                            editingSnippet.enabled
                              ? 'bg-amber-500/20 text-amber-400 hover:bg-amber-500/30'
                              : 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
                          }`}
                        >
                          {editingSnippet.enabled ? 'Disable' : 'Enable'}
                        </button>
                        {editingSnippet.is_builtin ? (
                          <button
                            onClick={() => handleReset(editingSnippet)}
                            className="px-3 py-1.5 bg-white/5 hover:bg-white/10 text-[var(--text-muted)] rounded-lg text-sm font-medium transition-colors"
                          >
                            Reset to Default
                          </button>
                        ) : (
                          <button
                            onClick={() => handleDelete(editingSnippet)}
                            className="px-3 py-1.5 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg text-sm font-medium transition-colors"
                          >
                            Delete
                          </button>
                        )}
                      </>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      onClick={resetForm}
                      className="px-4 py-2 bg-white/5 hover:bg-white/10 text-[var(--text-primary)] rounded-lg text-sm font-medium transition-colors"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={handleSave}
                      className="px-4 py-2 bg-indigo-500 hover:bg-indigo-600 text-white rounded-lg text-sm font-medium transition-colors"
                    >
                      {isCreating ? 'Create Snippet' : 'Save Changes'}
                    </button>
                  </div>
                </div>
              </>
            ) : (
              <div className="flex-1 flex flex-col items-center justify-center text-[var(--text-muted)]">
                <svg className="w-16 h-16 mb-4 opacity-30" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                </svg>
                <p className="text-sm mb-1">Select a snippet to edit</p>
                <p className="text-xs">or create a new one</p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
