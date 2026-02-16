// Minimal space selector - bottom dots with modals for create/edit
import { useState, useRef, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { useAppStore } from '../store';
import type { CreateSpaceInput, UpdateSpaceInput } from '../types';
import { save, open, ask } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';

// Arc-style space colors
const SPACE_COLORS = [
  '#C4B5FD', // pastel purple (violet-300)
  '#93C5FD', // pastel blue (blue-300)
  '#67E8F9', // pastel cyan (cyan-300)
  '#86EFAC', // pastel green (green-300)
  '#FDE047', // pastel yellow (yellow-300)
  '#FDBA74', // pastel orange (orange-300)
  '#F9A8D4', // pastel pink (pink-300)
  '#FCA5A5', // pastel red (red-300)
  '#A5F3FC', // light cyan (cyan-200)
  '#E9D5FF', // light purple (purple-200)
];

function getSpaceColor(index: number, customColor?: string | null): string {
  if (customColor) return customColor;
  return SPACE_COLORS[index % SPACE_COLORS.length];
}

// Connection form state interface
interface ConnectionFormState {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  trustCert: boolean;
  encrypt: boolean;
}

const emptyConnection: ConnectionFormState = {
  host: '',
  port: '1433',
  database: '',
  username: '',
  password: '',
  trustCert: true,
  encrypt: false,
};

export function SpacesSelector() {
  const {
    spaces,
    activeSpaceId,
    spacesLoading,
    setActiveSpace,
    createSpace,
    updateSpace,
    deleteSpace,
    testConnection,
  } = useAppStore();

  const [showMenu, setShowMenu] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [isEditing, setIsEditing] = useState<string | null>(null);
  const [newName, setNewName] = useState('');
  const [editName, setEditName] = useState('');
  const [selectedColor, setSelectedColor] = useState(SPACE_COLORS[0]);
  const [connection, setConnection] = useState<ConnectionFormState>(emptyConnection);
  const [isTesting, setIsTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [showSettingsMenu, setShowSettingsMenu] = useState(false);
  const settingsMenuRef = useRef<HTMLDivElement>(null);


  // Close menu when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(null);
      }
      if (settingsMenuRef.current && !settingsMenuRef.current.contains(e.target as Node)) {
        setShowSettingsMenu(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  // Focus input when creating/editing
  useEffect(() => {
    if ((isCreating || isEditing) && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isCreating, isEditing]);

  const handleCreate = async () => {
    if (!newName.trim()) return;

    const input: CreateSpaceInput = {
      name: newName.trim(),
      color: selectedColor,
      // Include connection if provided
      ...(connection.host && connection.database ? {
        connection_host: connection.host,
        connection_port: parseInt(connection.port) || 1433,
        connection_database: connection.database,
        connection_username: connection.username || undefined,
        connection_password: connection.password || undefined,
        connection_trust_cert: connection.trustCert,
        connection_encrypt: connection.encrypt,
      } : {}),
    };

    const space = await createSpace(input);
    setNewName('');
    setConnection(emptyConnection);
    setIsCreating(false);
    setSelectedColor(SPACE_COLORS[(spaces.length + 1) % SPACE_COLORS.length]);
    setTestResult(null);
    if (space) await setActiveSpace(space.id);
  };

  const handleEdit = async (id: string) => {
    if (!editName.trim()) return;

    const input: UpdateSpaceInput = {
      name: editName.trim(),
      color: selectedColor,
      // Include connection if provided
      ...(connection.host && connection.database ? {
        connection_host: connection.host,
        connection_port: parseInt(connection.port) || 1433,
        connection_database: connection.database,
        connection_username: connection.username || undefined,
        connection_password: connection.password || undefined,
        connection_trust_cert: connection.trustCert,
        connection_encrypt: connection.encrypt,
      } : {}),
    };

    await updateSpace(id, input);
    setIsEditing(null);
    setEditName('');
    setConnection(emptyConnection);
    setTestResult(null);
  };

  const handleTestConnection = async () => {
    if (!connection.host || !connection.database) return;

    setIsTesting(true);
    setTestResult(null);

    const success = await testConnection(
      connection.host,
      parseInt(connection.port) || 1433,
      connection.database,
      connection.username,
      connection.password,
      connection.trustCert,
      connection.encrypt
    );

    setTestResult(success ? 'success' : 'error');
    setIsTesting(false);
  };

  const handleDelete = async (id: string) => {
    setShowMenu(null);
    if (confirm('Delete this space and all its tabs?')) {
      await deleteSpace(id);
    }
  };

  const startEditing = (space: typeof spaces[0], index: number) => {
    setIsEditing(space.id);
    setEditName(space.name);
    setSelectedColor(getSpaceColor(index, space.color));
    // Load existing connection data
    setConnection({
      host: space.connection_host || '',
      port: String(space.connection_port || 1433),
      database: space.connection_database || '',
      username: space.connection_username || '',
      password: '', // Password is not returned from backend
      trustCert: space.connection_trust_cert ?? true,
      encrypt: space.connection_encrypt ?? false,
    });
    setShowMenu(null);
    setTestResult(null);
  };

  const handleExport = async () => {
    setShowSettingsMenu(false);
    try {
      const filePath = await save({
        title: 'Export Database',
        defaultPath: 'larik-backup.db',
        filters: [{ name: 'Database', extensions: ['db'] }],
      });

      if (filePath) {
        await invoke('export_database', { destination: filePath });
        // show toast
        useAppStore.getState().addToast({ type: 'success', message: 'Database exported successfully!' });
      }
    } catch (err) {
      console.error('Export failed:', err);
      useAppStore.getState().addToast({ type: 'error', message: `Export failed: ${err}` });
    }
  };

  const handleImport = async () => {
    setShowSettingsMenu(false);
    const confirmed = await ask(
      'This will replace all your current data and restart the application. Are you sure you want to continue?',
      { title: 'Import Database', kind: 'warning' }
    );

    if (confirmed) {
      try {
        const selected = await open({
          title: 'Import Database',
          multiple: false,
          filters: [{ name: 'Database', extensions: ['db'] }],
        });

        if (selected && !Array.isArray(selected)) {
          await invoke('import_database', { source: selected });
          // App will restart, so no success toast needed here.
        }
      } catch (err) {
        console.error('Import failed:', err);
        useAppStore.getState().addToast({ type: 'error', message: `Import failed: ${err}` });
      }
    }
  };

  if (spacesLoading && spaces.length === 0) {
    return (
      <div className="px-2 py-2 border-t border-white/5">
        <div className="flex items-center justify-center gap-2">
          {[1, 2, 3].map(i => (
            <div key={i} className="w-2.5 h-2.5 rounded-full bg-white/10 animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  return (
    <>
      {/* Bottom bar with space dots */}
      <div className="mt-auto px-2 py-2 border-t border-white/5">
        <div className="flex items-center justify-center gap-2">
          {spaces.map((space, index) => {
            const color = getSpaceColor(index, space.color);
            const isActive = space.id === activeSpaceId;

            return (
              <div key={space.id} className="relative" ref={showMenu === space.id ? menuRef : null}>
                <button
                  onClick={() => setActiveSpace(space.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setShowMenu(showMenu === space.id ? null : space.id);
                  }}
                  className={`
                    w-2.5 h-2.5 rounded-full transition-all duration-200 ease-out
                    ${isActive
                      ? 'scale-125 ring-2 ring-white/30 ring-offset-1 ring-offset-[var(--sidebar-bg)]'
                      : 'opacity-50 hover:opacity-100 hover:scale-110'
                    }
                  `}
                  style={{ backgroundColor: color }}
                  title={space.name}
                />

                {/* Context menu */}
                {showMenu === space.id && (
                  <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 w-40 bg-[var(--bg-tertiary)] border border-[var(--border-color)] rounded-lg shadow-xl z-50 py-1 animate-fade-in">
                    <button
                      onClick={() => startEditing(space, index)}
                      className="w-full px-3 py-2 text-left text-sm hover:bg-white/5 flex items-center gap-2"
                    >
                      <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                      </svg>
                      Edit Space
                    </button>
                    <button
                      onClick={() => handleDelete(space.id)}
                      className="w-full px-3 py-2 text-left text-sm hover:bg-red-500/20 text-red-400 flex items-center gap-2"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                      </svg>
                      Delete
                    </button>
                  </div>
                )}
              </div>
            );
          })}

          {/* Settings button */}
          <div className="relative" ref={settingsMenuRef}>
            <button
              onClick={() => setShowSettingsMenu(!showSettingsMenu)}
              className="w-2.5 h-2.5 rounded-full flex items-center justify-center
                            hover:bg-white/10 transition-all duration-200
                            opacity-50 hover:opacity-100"
              title="Settings"
            >
              <svg xmlns="http://www.w3.org/2000/svg" className="h-2.5 w-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 5v.01M12 12v.01M12 19v.01M12 6a1 1 0 110-2 1 1 0 010 2zm0 7a1 1 0 110-2 1 1 0 010 2zm0 7a1 1 0 110-2 1 1 0 010 2z" />
              </svg>
            </button>

            {/* Settings menu */}
            {showSettingsMenu && (
              <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 w-48 bg-[var(--bg-tertiary)] border border-[var(--border-color)] rounded-lg shadow-xl z-50 py-1 animate-fade-in">
                <div className="px-3 py-1.5 text-xs font-semibold text-[var(--text-muted)] border-b border-[var(--border-color)]">
                  Actions
                </div>
                <button
                  onClick={() => {
                    setIsCreating(true);
                    setShowSettingsMenu(false);
                  }}
                  className="w-full px-3 py-2 text-left text-sm hover:bg-white/5 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                  </svg>
                  New Space
                </button>
                <div className="px-3 py-1.5 text-xs font-semibold text-[var(--text-muted)] border-y border-[var(--border-color)]">
                  App Data
                </div>
                <button
                  onClick={handleImport}
                  className="w-full px-3 py-2 text-left text-sm hover:bg-white/5 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" /></svg>
                  Import Data...
                </button>
                <button
                  onClick={handleExport}
                  className="w-full px-3 py-2 text-left text-sm hover:bg-white/5 flex items-center gap-2"
                >
                  <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" /></svg>
                  Export Data...
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Create space modal */}
      {isCreating && createPortal(
        <div className="fixed inset-0 flex items-center justify-center pointer-events-none outline-none p-4" style={{ zIndex: 9999 }}>
          <div
            className="bg-black/40 absolute inset-0 pointer-events-auto"
            onClick={() => {
              setIsCreating(false);
              setNewName('');
              setConnection(emptyConnection);
              setTestResult(null);
            }}
          />
          <div className="relative pointer-events-auto bg-[var(--bg-secondary)] p-5 rounded-2xl shadow-2xl w-full max-w-md max-h-[90vh] overflow-y-auto animate-fade-in" style={{ zIndex: 10000 }}>
            <h3 className="text-lg font-semibold mb-4">New Space</h3>
            <input
              ref={inputRef}
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  setIsCreating(false);
                  setNewName('');
                  setConnection(emptyConnection);
                }
              }}
              placeholder="Space name..."
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg mb-4 focus:border-[var(--accent-color)] outline-none"
            />

            {/* Color picker */}
            <div className="mb-4">
              <label className="text-sm text-[var(--text-secondary)] mb-2 block">Color</label>
              <div className="flex gap-2 flex-wrap">
                {SPACE_COLORS.map(color => (
                  <button
                    key={color}
                    onClick={() => setSelectedColor(color)}
                    className={`w-7 h-7 rounded-full transition-all ${selectedColor === color ? 'ring-2 ring-white ring-offset-2 ring-offset-[var(--bg-secondary)] scale-110' : ''
                      }`}
                    style={{ backgroundColor: color }}
                  />
                ))}
              </div>
            </div>

            {/* Connection form */}
            <div className="mb-4 p-3 bg-white/5 rounded-lg">
              <h4 className="text-sm font-medium mb-3 flex items-center gap-2">
                <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2" />
                </svg>
                Database Connection <span className="text-xs text-[var(--text-secondary)] font-normal">(optional)</span>
              </h4>

              <div className="grid grid-cols-3 gap-2 mb-2">
                <div className="col-span-2">
                  <input
                    type="text"
                    value={connection.host}
                    onChange={(e) => setConnection(c => ({ ...c, host: e.target.value }))}
                    placeholder="Host / Server"
                    className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                </div>
                <div>
                  <input
                    type="text"
                    value={connection.port}
                    onChange={(e) => setConnection(c => ({ ...c, port: e.target.value }))}
                    placeholder="Port"
                    className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                </div>
              </div>

              <input
                type="text"
                value={connection.database}
                onChange={(e) => setConnection(c => ({ ...c, database: e.target.value }))}
                placeholder="Database"
                className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm mb-2 focus:border-[var(--accent-color)] outline-none"
              />

              <div className="grid grid-cols-2 gap-2 mb-2">
                <input
                  type="text"
                  value={connection.username}
                  onChange={(e) => setConnection(c => ({ ...c, username: e.target.value }))}
                  placeholder="Username"
                  className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                />
                <input
                  type="password"
                  value={connection.password}
                  onChange={(e) => setConnection(c => ({ ...c, password: e.target.value }))}
                  placeholder="Password"
                  className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                />
              </div>

              <div className="flex gap-4 mb-3 text-xs">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={connection.trustCert}
                    onChange={(e) => setConnection(c => ({ ...c, trustCert: e.target.checked }))}
                    className="rounded"
                  />
                  Trust Certificate
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={connection.encrypt}
                    onChange={(e) => setConnection(c => ({ ...c, encrypt: e.target.checked }))}
                    className="rounded"
                  />
                  Encrypt
                </label>
              </div>

              {/* Test connection button */}
              {connection.host && connection.database && (
                <div className="flex items-center gap-2">
                  <button
                    onClick={handleTestConnection}
                    disabled={isTesting}
                    className="text-xs px-2 py-1 rounded bg-white/10 hover:bg-white/20 disabled:opacity-50"
                  >
                    {isTesting ? 'Testing...' : 'Test Connection'}
                  </button>
                  {testResult === 'success' && <span className="text-xs text-green-400">✓ Connected!</span>}
                  {testResult === 'error' && <span className="text-xs text-red-400">✗ Failed</span>}
                </div>
              )}
            </div>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setIsCreating(false);
                  setNewName('');
                  setConnection(emptyConnection);
                  setTestResult(null);
                }}
                className="px-4 py-2 rounded-lg hover:bg-white/5 text-[var(--text-secondary)]"
              >
                Cancel
              </button>
              <button
                onClick={handleCreate}
                disabled={!newName.trim()}
                className="px-4 py-2 rounded-lg bg-[var(--accent-color)] hover:bg-[var(--accent-hover)] text-white font-medium disabled:opacity-50"
              >
                Create
              </button>
            </div>
          </div>
        </div>,
        document.body
      )}

      {/* Edit space modal */}
      {isEditing && createPortal(
        <div className="fixed inset-0 flex items-center justify-center pointer-events-none outline-none p-4" style={{ zIndex: 9999 }}>
          <div
            className="bg-black/40 absolute inset-0 pointer-events-auto"
            onClick={() => {
              setIsEditing(null);
              setEditName('');
              setConnection(emptyConnection);
              setTestResult(null);
            }}
          />
          <div className="relative pointer-events-auto bg-[var(--bg-secondary)] p-5 rounded-2xl shadow-2xl w-full max-w-md max-h-[90vh] overflow-y-auto animate-fade-in" style={{ zIndex: 10000 }}>
            <h3 className="text-lg font-semibold mb-4">Edit Space</h3>
            <input
              ref={inputRef}
              type="text"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  setIsEditing(null);
                  setEditName('');
                  setConnection(emptyConnection);
                }
              }}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg mb-4 focus:border-[var(--accent-color)] outline-none"
            />

            {/* Color picker */}
            <div className="mb-4">
              <label className="text-sm text-[var(--text-secondary)] mb-2 block">Color</label>
              <div className="flex gap-2 flex-wrap">
                {SPACE_COLORS.map(color => (
                  <button
                    key={color}
                    onClick={() => setSelectedColor(color)}
                    className={`w-7 h-7 rounded-full transition-all ${selectedColor === color ? 'ring-2 ring-white ring-offset-2 ring-offset-[var(--bg-secondary)] scale-110' : ''
                      }`}
                    style={{ backgroundColor: color }}
                  />
                ))}
              </div>
            </div>

            {/* Connection form */}
            <div className="mb-4 p-3 bg-white/5 rounded-lg">
              <h4 className="text-sm font-medium mb-3 flex items-center gap-2">
                <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2" />
                </svg>
                Database Connection
              </h4>

              <div className="grid grid-cols-3 gap-2 mb-2">
                <div className="col-span-2">
                  <input
                    type="text"
                    value={connection.host}
                    onChange={(e) => setConnection(c => ({ ...c, host: e.target.value }))}
                    placeholder="Host / Server"
                    className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                </div>
                <div>
                  <input
                    type="text"
                    value={connection.port}
                    onChange={(e) => setConnection(c => ({ ...c, port: e.target.value }))}
                    placeholder="Port"
                    className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                </div>
              </div>

              <input
                type="text"
                value={connection.database}
                onChange={(e) => setConnection(c => ({ ...c, database: e.target.value }))}
                placeholder="Database"
                className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm mb-2 focus:border-[var(--accent-color)] outline-none"
              />

              <div className="grid grid-cols-2 gap-2 mb-2">
                <input
                  type="text"
                  value={connection.username}
                  onChange={(e) => setConnection(c => ({ ...c, username: e.target.value }))}
                  placeholder="Username"
                  className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                />
                <input
                  type="password"
                  value={connection.password}
                  onChange={(e) => setConnection(c => ({ ...c, password: e.target.value }))}
                  placeholder="Password (leave blank to keep)"
                  className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                />
              </div>

              <div className="flex gap-4 mb-3 text-xs">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={connection.trustCert}
                    onChange={(e) => setConnection(c => ({ ...c, trustCert: e.target.checked }))}
                    className="rounded"
                  />
                  Trust Certificate
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={connection.encrypt}
                    onChange={(e) => setConnection(c => ({ ...c, encrypt: e.target.checked }))}
                    className="rounded"
                  />
                  Encrypt
                </label>
              </div>

              {/* Test connection button */}
              {connection.host && connection.database && (
                <div className="flex items-center gap-2">
                  <button
                    onClick={handleTestConnection}
                    disabled={isTesting}
                    className="text-xs px-2 py-1 rounded bg-white/10 hover:bg-white/20 disabled:opacity-50"
                  >
                    {isTesting ? 'Testing...' : 'Test Connection'}
                  </button>
                  {testResult === 'success' && <span className="text-xs text-green-400">✓ Connected!</span>}
                  {testResult === 'error' && <span className="text-xs text-red-400">✗ Failed</span>}
                </div>
              )}
            </div>

            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setIsEditing(null);
                  setEditName('');
                  setConnection(emptyConnection);
                  setTestResult(null);
                }}
                className="px-4 py-2 rounded-lg hover:bg-white/5 text-[var(--text-secondary)]"
              >
                Cancel
              </button>
              <button
                onClick={() => handleEdit(isEditing)}
                className="px-4 py-2 rounded-lg bg-[var(--accent-color)] hover:bg-[var(--accent-hover)] text-white font-medium"
              >
                Save
              </button>
            </div>
          </div>
        </div>,
        document.body
      )}
    </>
  );
}
