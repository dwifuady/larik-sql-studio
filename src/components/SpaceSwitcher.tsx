// Space switcher - expandable dropdown under the database selector
import { useState, useRef, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { useAppStore } from '../store';
import type { UpdateSpaceInput } from '../types';
import { spaceHasConnection } from '../types';
import { save, open, ask } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';

import { SPACE_COLORS } from '@/utils/spaceColors';

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

export function SpaceSwitcher() {
  const spaces = useAppStore(s => s.spaces);
  const activeSpaceId = useAppStore(s => s.activeSpaceId);
  const setActiveSpace = useAppStore(s => s.setActiveSpace);
  const updateSpace = useAppStore(s => s.updateSpace);
  const deleteSpace = useAppStore(s => s.deleteSpace);
  const testConnection = useAppStore(s => s.testConnection);
  const setCreateSpaceModalOpen = useAppStore(s => s.setCreateSpaceModalOpen);
  const isConnected = useAppStore(s => s.spaceConnectionStatus?.is_connected ?? false);
  const isConnecting = useAppStore(s => s.isConnecting);
  const connectToSpace = useAppStore(s => s.connectToSpace);
  const disconnectFromSpace = useAppStore(s => s.disconnectFromSpace);

  const [isOpen, setIsOpen] = useState(false);
  const [isEditing, setIsEditing] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [selectedColor, setSelectedColor] = useState(SPACE_COLORS[0]);
  const [connection, setConnection] = useState<ConnectionFormState>(emptyConnection);
  const [isTesting, setIsTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const activeSpace = spaces.find(s => s.id === activeSpaceId);
  const spaceColor = activeSpace?.color || '#6366f1';

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  // Focus input when editing
  useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isEditing]);

  if (!activeSpace) return null;

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
    setIsOpen(false);
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
    setIsOpen(false);
    setTestResult(null);
  };

  const handleExport = async () => {
    setIsOpen(false);
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
    setIsOpen(false);
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

  return (
    <div ref={containerRef} className="relative group">
      {/* Trigger row: active space + connection controls */}
      <div
        className="flex items-center gap-1 bg-transparent hover:bg-[var(--bg-hover)] rounded-md transition-all pl-1.5 pr-0.5 h-6"
        style={{
          boxShadow: isConnected ? `0 0 0 1px ${spaceColor}10` : 'none'
        }}
      >
        <button
          onClick={() => setIsOpen(!isOpen)}
          className="flex-1 min-w-0 h-full flex items-center gap-1.5 text-left"
          title="Switch space"
          aria-expanded={isOpen}
        >
          <svg
            className={`w-3.5 h-3.5 flex-shrink-0 opacity-40 group-hover:opacity-100 transition-transform ${isOpen ? 'rotate-90' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: spaceColor }} />
          <span className="text-xs font-semibold text-[var(--text-primary)] truncate">{activeSpace.name}</span>
        </button>

        {spaceHasConnection(activeSpace) && (
          <div className="flex items-center gap-1">
            {/* Connect/Disconnect Button - Only visible on hover or if disconnected */}
            <button
              onClick={() => {
                isConnected ? disconnectFromSpace() : connectToSpace();
              }}
              disabled={isConnecting}
              className={`p-1 rounded hover:bg-[var(--bg-hover)] transition-all ${isConnected
                ? 'text-[var(--text-muted)] hover:text-red-400 opacity-0 group-hover:opacity-100'
                : 'text-[var(--text-muted)] hover:text-green-400 opacity-100'
                }`}
              title={isConnected ? 'Disconnect' : 'Connect'}
            >
              <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
            </button>

            {/* Connection Status Badge */}
            <span
              className={`text-[9px] font-medium px-1.5 py-0.5 rounded-full transition-colors ${
                isConnected
                  ? 'bg-green-500/20 text-green-400'
                  : 'bg-[var(--bg-hover)] text-[var(--text-muted)]'
              }`}
            >
              {isConnected ? 'Connected' : 'Disconnected'}
            </span>
          </div>
        )}
      </div>

      {/* Expandable space list */}
      {isOpen && (
        <div
          className="absolute top-full left-0 right-0 mt-1 z-[20] py-1 rounded-md border border-[var(--border-color)] shadow-2xl overflow-hidden animate-in fade-in slide-in-from-top-1 duration-150 bg-[var(--bg-secondary)] backdrop-blur-xl"
          style={{ maxHeight: '280px', overflowY: 'auto' }}
        >
          <div className="px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--text-muted)]">
            Spaces
          </div>

          {spaces.map((space, index) => {
            const isActive = space.id === activeSpaceId;
            return (
              <div key={space.id} className="group/row relative">
                <button
                  onClick={() => {
                    setActiveSpace(space.id);
                    setIsOpen(false);
                  }}
                  className={`w-full flex items-center gap-2 px-2.5 py-1.5 pr-6 text-left text-xs transition-colors ${
                    isActive
                      ? 'bg-[var(--bg-active)] text-[var(--text-primary)]'
                      : 'text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'
                  }`}
                >
                  <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: getSpaceColor(index, space.color) }} />
                  <span className="truncate flex-1">{space.name}</span>
                  {isActive && (
                    <svg
                      className="w-3.5 h-3.5 flex-shrink-0 mr-1 text-[var(--text-muted)] transition-opacity group-hover/row:opacity-0"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                    </svg>
                  )}
                </button>
                {/* Hover actions - overlay so the row keeps full width */}
                <div className="absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5 opacity-0 group-hover/row:opacity-100 transition-opacity">
                  <button
                    onClick={() => startEditing(space, index)}
                    className="p-1 rounded hover:bg-white/10 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                    title="Edit Space"
                  >
                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                    </svg>
                  </button>
                  <button
                    onClick={() => handleDelete(space.id)}
                    className="p-1 rounded hover:bg-red-500/20 text-[var(--text-muted)] hover:text-red-400"
                    title="Delete"
                  >
                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                    </svg>
                  </button>
                </div>
              </div>
            );
          })}

          <div className="my-1 mx-2 border-t border-[var(--border-color)] opacity-50" />

          {/* New Space */}
          <button
            onClick={() => {
              setCreateSpaceModalOpen(true);
              setIsOpen(false);
            }}
            className="w-full px-2.5 py-1.5 text-left text-xs flex items-center gap-2 text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
          >
            <svg className="w-3 h-3 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
            </svg>
            New Space
          </button>

          <div className="px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-[var(--text-muted)] border-t border-[var(--border-color)] mt-1">
            App Data
          </div>
          <button
            onClick={handleImport}
            className="w-full px-2.5 py-1.5 text-left text-xs flex items-center gap-2 text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
          >
            <svg className="w-3 h-3 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" /></svg>
            Import Data...
          </button>
          <button
            onClick={handleExport}
            className="w-full px-2.5 py-1.5 text-left text-xs flex items-center gap-2 text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
          >
            <svg className="w-3 h-3 flex-shrink-0 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" /></svg>
            Export Data...
          </button>
        </div>
      )}

      {/* Edit space modal */}
      {
        isEditing && createPortal(
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
            <div className="relative pointer-events-auto bg-[var(--bg-secondary)] p-4 rounded-xl shadow-2xl w-full max-w-md max-h-[90vh] overflow-y-auto animate-fade-in" style={{ zIndex: 10000 }}>
              <h3 className="text-base font-semibold mb-3">Edit Space</h3>
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
                className="w-full px-2.5 py-1.5 bg-white/5 border border-white/10 rounded-md mb-3 focus:border-[var(--accent-color)] outline-none text-sm"
              />

              {/* Color picker */}
              <div className="mb-3">
                <label className="text-sm text-[var(--text-secondary)] mb-1.5 block">Color</label>
                <div className="flex gap-1.5 flex-wrap">
                  {SPACE_COLORS.map(color => (
                    <button
                      key={color}
                      onClick={() => setSelectedColor(color)}
                      className={`w-6 h-6 rounded-full transition-all ${selectedColor === color ? 'ring-2 ring-white ring-offset-2 ring-offset-[var(--bg-secondary)] scale-110' : ''
                        }`}
                      style={{ backgroundColor: color }}
                    />
                  ))}
                </div>
              </div>

              {/* Connection form */}
              <div className="mb-3 p-2.5 bg-white/5 rounded-md">
                <h4 className="text-sm font-medium mb-2 flex items-center gap-1.5">
                  <svg className="w-3.5 h-3.5 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2" />
                  </svg>
                  Database Connection
                </h4>

                <div className="grid grid-cols-3 gap-1.5 mb-1.5">
                  <div className="col-span-2">
                    <input
                      type="text"
                      value={connection.host}
                      onChange={(e) => setConnection(c => ({ ...c, host: e.target.value }))}
                      placeholder="Host / Server"
                      className="w-full px-2 py-1 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                    />
                  </div>
                  <div>
                    <input
                      type="text"
                      value={connection.port}
                      onChange={(e) => setConnection(c => ({ ...c, port: e.target.value }))}
                      placeholder="Port"
                      className="w-full px-2 py-1 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                    />
                  </div>
                </div>

                <input
                  type="text"
                  value={connection.database}
                  onChange={(e) => setConnection(c => ({ ...c, database: e.target.value }))}
                  placeholder="Database"
                  className="w-full px-2 py-1 bg-white/5 border border-white/10 rounded text-sm mb-1.5 focus:border-[var(--accent-color)] outline-none"
                />

                <div className="grid grid-cols-2 gap-1.5 mb-1.5">
                  <input
                    type="text"
                    value={connection.username}
                    onChange={(e) => setConnection(c => ({ ...c, username: e.target.value }))}
                    placeholder="Username"
                    className="w-full px-2 py-1 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                  <input
                    type="password"
                    value={connection.password}
                    onChange={(e) => setConnection(c => ({ ...c, password: e.target.value }))}
                    placeholder="Password (leave blank to keep)"
                    className="w-full px-2 py-1 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                  />
                </div>

                <div className="flex gap-3 mb-2 text-xs">
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
                  <div className="flex items-center gap-1.5">
                    <button
                      onClick={handleTestConnection}
                      disabled={isTesting}
                      className="text-xs px-2 py-0.5 rounded bg-white/10 hover:bg-white/20 disabled:opacity-50"
                    >
                      {isTesting ? 'Testing...' : 'Test Connection'}
                    </button>
                    {testResult === 'success' && <span className="text-xs text-green-400">✓ Connected!</span>}
                    {testResult === 'error' && <span className="text-xs text-red-400">✗ Failed</span>}
                  </div>
                )}
              </div>

              <div className="flex justify-end gap-1.5">
                <button
                  onClick={() => {
                    setIsEditing(null);
                    setEditName('');
                    setConnection(emptyConnection);
                    setTestResult(null);
                  }}
                  className="px-3 py-1.5 rounded-md hover:bg-white/5 text-[var(--text-secondary)] text-sm"
                >
                  Cancel
                </button>
                <button
                  onClick={() => handleEdit(isEditing)}
                  className="px-3 py-1.5 rounded-md bg-[var(--accent-color)] hover:bg-[var(--accent-hover)] text-white font-medium text-sm"
                >
                  Save
                </button>
              </div>
            </div>
          </div>,
          document.body
        )
      }
    </div>
  );
}
