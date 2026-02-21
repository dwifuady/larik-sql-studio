// Create space modal - extracted from SpacesSelector for reusability
import { useState, useRef, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../store';
import type { CreateSpaceInput, DatabaseType } from '../types';

// Arc-style space colors
const SPACE_COLORS = [
  '#8b5cf6', // purple
  '#3b82f6', // blue
  '#06b6d4', // cyan
  '#22c55e', // green
  '#eab308', // yellow
  '#f97316', // orange
  '#ec4899', // pink
  '#ef4444', // red
];

// Connection form state interface for MS-SQL
interface ConnectionFormState {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  trustCert: boolean;
  encrypt: boolean;
}

// Connection form state interface for PostgreSQL
interface PostgresConnectionFormState {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  sslmode: 'prefer' | 'require' | 'verify-ca' | 'verify-full' | 'disable';
}

// Connection form state interface for MySQL
interface MysqlConnectionFormState {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  ssl_enabled: boolean;
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

const emptyPostgresConnection: PostgresConnectionFormState = {
  host: '',
  port: '5432',
  database: '',
  username: '',
  password: '',
  sslmode: 'prefer',
};

const emptyMysqlConnection: MysqlConnectionFormState = {
  host: '',
  port: '3306',
  database: '',
  username: '',
  password: '',
  ssl_enabled: false,
};

// SQLite form state
interface SqliteFormState {
  filePath: string;
}

const emptySqliteForm: SqliteFormState = {
  filePath: '',
};

interface CreateSpaceModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSpaceCreated?: () => void;
}

export function CreateSpaceModal({ isOpen, onClose, onSpaceCreated }: CreateSpaceModalProps) {
  const {
    spaces,
    createSpace,
    setActiveSpace,
    testConnection,
  } = useAppStore();

  const [newName, setNewName] = useState('');
  const [selectedColor, setSelectedColor] = useState(SPACE_COLORS[0]);
  const [databaseType, setDatabaseType] = useState<DatabaseType | null>(null);
  const [connection, setConnection] = useState<ConnectionFormState | PostgresConnectionFormState | MysqlConnectionFormState>(emptyConnection);
  const [sqliteForm, setSqliteForm] = useState<SqliteFormState>(emptySqliteForm);
  const [isTesting, setIsTesting] = useState(false);
  const [testResult, setTestResult] = useState<'success' | 'error' | 'warning' | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset connection form when database type changes
  useEffect(() => {
    setTestResult(null);
    if (databaseType === null) {
      setConnection(emptyConnection);
    } else if (databaseType === 'mssql') {
      setConnection(emptyConnection);
    } else if (databaseType === 'postgresql') {
      setConnection(emptyPostgresConnection);
    } else if (databaseType === 'mysql') {
      setConnection(emptyMysqlConnection);
    } else if (databaseType === 'sqlite') {
      setSqliteForm(emptySqliteForm);
    }
  }, [databaseType]);

  // Focus input when modal opens
  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isOpen]);

  const handleBrowseSqliteFile = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: 'SQLite Database', extensions: ['db', 'sqlite', 'sqlite3', 's3db'] }],
        title: 'Open SQLite Database',
      });
      if (typeof selected === 'string') {
        setSqliteForm({ filePath: selected });
      }
    } catch (e) {
      console.error('File dialog failed:', e);
    }
  };

  const handleCreate = async () => {
    if (!newName.trim()) return;

    // Construct the input object based on database type
    let input: CreateSpaceInput = {
      name: newName.trim(),
      color: selectedColor,
      database_type: databaseType,
    };

    if (databaseType === 'sqlite') {
      // For SQLite, `connection_database` holds the file path
      input = {
        ...input,
        connection_database: sqliteForm.filePath || undefined,
      };
    } else if (databaseType) {
      // Map common fields
      input = {
        ...input,
        connection_host: connection.host,
        connection_port: parseInt(connection.port) || undefined,
        connection_database: connection.database,
        connection_username: connection.username || undefined,
        connection_password: connection.password || undefined,
      };

      // Map specific fields
      if (databaseType === 'mssql') {
        const mssqlConfig = connection as ConnectionFormState;
        input = {
          ...input,
          connection_port: parseInt(mssqlConfig.port) || 1433,
          connection_trust_cert: mssqlConfig.trustCert,
          connection_encrypt: mssqlConfig.encrypt,
        };
      } else if (databaseType === 'postgresql') {
        const pgConfig = connection as PostgresConnectionFormState;
        input = {
          ...input,
          connection_port: parseInt(pgConfig.port) || 5432,
          postgres_sslmode: pgConfig.sslmode,
        };
      } else if (databaseType === 'mysql') {
        const mysqlConfig = connection as MysqlConnectionFormState;
        input = {
          ...input,
          connection_port: parseInt(mysqlConfig.port) || 3306,
          mysql_ssl_enabled: mysqlConfig.ssl_enabled,
        };
      }
    }

    const space = await createSpace(input);
    setNewName('');
    setDatabaseType(null);
    setConnection(emptyConnection);
    setSqliteForm(emptySqliteForm);
    setSelectedColor(SPACE_COLORS[(spaces.length + 1) % SPACE_COLORS.length]);
    setTestResult(null);
    if (space) {
      await setActiveSpace(space.id);
      onSpaceCreated?.();
    }
    onClose();
  };

  const handleTestConnection = async () => {
    setIsTesting(true);
    setTestResult(null);

    try {
      if (databaseType === 'sqlite') {
        // For SQLite, just try to open the file — test via Tauri invoke directly
        const success = await testConnection(
          '', 0, sqliteForm.filePath, '', '', false, false, 'sqlite', undefined,
        );
        setTestResult(success ? 'success' : 'error');
      } else {
        if (!connection.host || !connection.database) {
          setIsTesting(false);
          return;
        }

        let trustCert = false;
        let encrypt = false;
        let sslMode: string | undefined = undefined;

        if (databaseType === 'mssql') {
          trustCert = (connection as ConnectionFormState).trustCert;
          encrypt = (connection as ConnectionFormState).encrypt;
        } else if (databaseType === 'postgresql') {
          sslMode = (connection as PostgresConnectionFormState).sslmode;
        }

        const success = await testConnection(
          connection.host,
          parseInt(connection.port) || (databaseType === 'postgresql' ? 5432 : 1433),
          connection.database,
          connection.username,
          connection.password,
          trustCert,
          encrypt,
          databaseType || 'mssql',
          sslMode,
        );
        setTestResult(success ? 'success' : 'error');
      }
    } catch (e) {
      console.error('Connection test failed:', e);
      setTestResult('error');
    }

    setIsTesting(false);
  };

  const handleClose = () => {
    setNewName('');
    setDatabaseType(null);
    setConnection(emptyConnection);
    setSqliteForm(emptySqliteForm);
    setSelectedColor(SPACE_COLORS[0]);
    setTestResult(null);
    onClose();
  };

  if (!isOpen) return null;

  return createPortal(
    <div className="fixed inset-0 flex items-center justify-center pointer-events-none outline-none p-4" style={{ zIndex: 9999 }}>
      <div
        className="bg-black/40 absolute inset-0 pointer-events-auto"
        onClick={handleClose}
      />
      <div className="relative pointer-events-auto bg-[var(--bg-secondary)] p-5 rounded-2xl shadow-2xl w-full max-w-md max-h-[90vh] overflow-y-auto animate-fade-in" style={{ zIndex: 10000 }}>
        <h3 className="text-lg font-semibold mb-4">New Space</h3>
        <input
          ref={inputRef}
          type="text"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && newName.trim()) {
              handleCreate();
            } else if (e.key === 'Escape') {
              handleClose();
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

        {/* Database Type Selector */}
        <div className="mb-4">
          <label className="text-sm text-[var(--text-secondary)] mb-2 block">Database Type</label>
          <div className="flex gap-2 flex-wrap">
            <button
              onClick={() => { setDatabaseType(null); setConnection(emptyConnection); }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all ${databaseType === null ? 'bg-[var(--accent-color)] text-white' : 'bg-white/5 border border-white/10 hover:bg-white/10'
                }`}
            >
              None
            </button>
            <button
              onClick={() => { setDatabaseType('sqlite'); setConnection(emptyConnection); }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all ${databaseType === 'sqlite' ? 'bg-[var(--accent-color)] text-white' : 'bg-white/5 border border-white/10 hover:bg-white/10'
                }`}
            >
              SQLite
            </button>
            <button
              onClick={() => { setDatabaseType('mssql'); setConnection(emptyConnection); }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all ${databaseType === 'mssql' ? 'bg-[var(--accent-color)] text-white' : 'bg-white/5 border border-white/10 hover:bg-white/10'
                }`}
            >
              MS SQL
            </button>
            <button
              onClick={() => { setDatabaseType('postgresql'); setConnection(emptyPostgresConnection); }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all ${databaseType === 'postgresql' ? 'bg-[var(--accent-color)] text-white' : 'bg-white/5 border border-white/10 hover:bg-white/10'
                }`}
            >
              PostgreSQL
            </button>
            <button
              onClick={() => { setDatabaseType('mysql'); setConnection(emptyMysqlConnection); }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all ${databaseType === 'mysql' ? 'bg-[var(--accent-color)] text-white' : 'bg-white/5 border border-white/10 hover:bg-white/10'
                }`}
            >
              MySQL
            </button>
          </div>
        </div>

        {/* SQLite file picker */}
        {databaseType === 'sqlite' && (
          <div className="mb-4 p-3 bg-white/5 rounded-lg">
            <h4 className="text-sm font-medium mb-3 flex items-center gap-2">
              <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
              </svg>
              SQLite Database File
              <span className="text-xs text-[var(--text-secondary)] font-normal">(optional)</span>
            </h4>

            <div className="flex gap-2 mb-2">
              <input
                type="text"
                value={sqliteForm.filePath}
                onChange={(e) => setSqliteForm({ filePath: e.target.value })}
                placeholder="/path/to/database.db"
                className="flex-1 px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none font-mono"
              />
              <button
                onClick={handleBrowseSqliteFile}
                className="px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-sm whitespace-nowrap transition-colors"
              >
                Browse…
              </button>
            </div>

            <p className="text-xs text-[var(--text-secondary)] mb-3">
              Supported: <span className="text-[var(--text-primary)]">.db</span>,{' '}
              <span className="text-[var(--text-primary)]">.sqlite</span>,{' '}
              <span className="text-[var(--text-primary)]">.sqlite3</span>. Leave blank to create an empty space.
            </p>

            {/* Test connection for SQLite */}
            {sqliteForm.filePath && (
              <div className="flex items-center gap-2">
                <button
                  onClick={handleTestConnection}
                  disabled={isTesting}
                  className="text-xs px-2 py-1 rounded bg-white/10 hover:bg-white/20 disabled:opacity-50 transition-colors"
                >
                  {isTesting ? 'Checking…' : 'Verify File'}
                </button>
                {testResult === 'success' && <span className="text-xs text-green-400">✓ File accessible</span>}
                {testResult === 'error' && <span className="text-xs text-red-400">✗ Cannot open file</span>}
              </div>
            )}
          </div>
        )}

        {/* Connection form - only show if database type is not SQLite and not None */}
        {databaseType !== null && databaseType !== 'sqlite' && (
          <div className="mb-4 p-3 bg-white/5 rounded-lg">
            <h4 className="text-sm font-medium mb-3 flex items-center gap-2">
              <svg className="w-4 h-4 opacity-60" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012-2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2 2" />
              </svg>
              {databaseType === 'mssql' ? 'MS-SQL' : databaseType === 'postgresql' ? 'PostgreSQL' : 'MySQL'} Connection
              <span className="text-xs text-[var(--text-secondary)] font-normal">(optional)</span>
            </h4>

            {/* Host/Port - MS-SQL, PostgreSQL, MySQL */}
            <div className="grid grid-cols-2 gap-2 mb-2">
              <input
                type="text"
                value={connection.host}
                onChange={(e) => setConnection(c => ({ ...c, host: e.target.value }))}
                placeholder="Host / Server"
                className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
              />
              <input
                type="text"
                value={connection.port}
                onChange={(e) => setConnection(c => ({ ...c, port: e.target.value }))}
                placeholder="Port"
                className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
              />
            </div>

            {/* Database */}
            <input
              type="text"
              value={connection.database}
              onChange={(e) => setConnection(c => ({ ...c, database: e.target.value }))}
              placeholder="Database name"
              className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm mb-2 focus:border-[var(--accent-color)] outline-none"
            />

            {/* Username/Password - MS-SQL, PostgreSQL, MySQL */}
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

            {/* Database-specific options */}
            {databaseType === 'mssql' && (
              <div className="flex gap-4 mb-3 text-xs">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={(connection as ConnectionFormState).trustCert}
                    onChange={(e) => setConnection(c => ({ ...(c as ConnectionFormState), trustCert: e.target.checked }))}
                    className="rounded"
                  />
                  Trust Certificate
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={(connection as ConnectionFormState).encrypt}
                    onChange={(e) => setConnection(c => ({ ...(c as ConnectionFormState), encrypt: e.target.checked }))}
                    className="rounded"
                  />
                  Encrypt
                </label>
              </div>
            )}

            {databaseType === 'postgresql' && (
              <div className="mb-3">
                <label className="text-sm text-[var(--text-secondary)] mb-2 block">SSL Mode</label>
                <select
                  value={(connection as PostgresConnectionFormState).sslmode}
                  onChange={(e) => setConnection(c => ({ ...(c as PostgresConnectionFormState), sslmode: e.target.value as any }))}
                  className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-sm focus:border-[var(--accent-color)] outline-none"
                >
                  <option value="prefer">Prefer</option>
                  <option value="require">Require</option>
                  <option value="verify-ca">Verify CA</option>
                  <option value="verify-full">Verify Full</option>
                  <option value="disable">Disable</option>
                </select>
              </div>
            )}

            {databaseType === 'mysql' && (
              <div className="mb-3">
                <label className="flex items-center gap-1.5 cursor-pointer text-xs">
                  <input
                    type="checkbox"
                    checked={(connection as MysqlConnectionFormState).ssl_enabled}
                    onChange={(e) => setConnection(c => ({ ...(c as MysqlConnectionFormState), ssl_enabled: e.target.checked }))}
                    className="rounded"
                  />
                  Enable SSL
                </label>
              </div>
            )}

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
                {testResult === 'warning' && <span className="text-xs text-yellow-400">⚠ Not Implemented</span>}
              </div>
            )}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={handleClose}
            className="px-4 py-2 rounded-lg hover:bg-white/5 text-[var(--text-secondary)]"
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            disabled={!newName.trim()}
            className="px-4 py-2 rounded-lg bg-[var(--accent-color)] hover:bg-[var(--accent-hover)] text-white font-medium disabled:opacity-50"
          >
            Create Space
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
