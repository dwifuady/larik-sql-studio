// Export Dialog Component (T037)
// Provides export options and progress UI for CSV/JSON export

import { useState, useCallback, useEffect } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { exportToCsv, exportToJson, exportToString } from '../api';
import type { QueryResult, ExportOptions, ExportProgress } from '../types';

interface ExportDialogProps {
  isOpen: boolean;
  onClose: () => void;
  result: QueryResult;
  spaceColor?: string;
}

export function ExportDialog({ isOpen, onClose, result, spaceColor }: ExportDialogProps) {
  const [format, setFormat] = useState<'csv' | 'json'>('csv');
  const [isExporting, setIsExporting] = useState(false);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [options, setOptions] = useState<ExportOptions>({
    include_headers: true,
    pretty_print: true,
    delimiter: ',',
    null_as_string: true,
  });

  // Reset state when dialog opens
  useEffect(() => {
    if (isOpen) {
      setProgress(null);
      setError(null);
      setIsExporting(false);
    }
  }, [isOpen]);

  const handleExport = useCallback(async () => {
    try {
      setIsExporting(true);
      setError(null);
      setProgress(null);

      // Open file save dialog
      const defaultName = `export_${new Date().toISOString().slice(0, 10)}.${format}`;
      const filters = format === 'csv' 
        ? [{ name: 'CSV Files', extensions: ['csv'] }]
        : [{ name: 'JSON Files', extensions: ['json'] }];

      const filePath = await save({
        defaultPath: defaultName,
        filters,
        title: `Export to ${format.toUpperCase()}`,
      });

      if (!filePath) {
        setIsExporting(false);
        return; // User cancelled
      }

      // Perform export
      const exportFn = format === 'csv' ? exportToCsv : exportToJson;
      const result_progress = await exportFn(filePath, result.columns, result.rows, options);

      setProgress(result_progress);
      
      if (result_progress.error) {
        setError(result_progress.error);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsExporting(false);
    }
  }, [format, options, result]);

  const handleCopyToClipboard = useCallback(async () => {
    try {
      setIsExporting(true);
      setError(null);

      const exportedString = await exportToString(format, result.columns, result.rows, options);
      await navigator.clipboard.writeText(exportedString);

      setProgress({
        rows_exported: result.rows.length,
        total_rows: result.rows.length,
        bytes_written: exportedString.length,
        is_complete: true,
        error: null,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsExporting(false);
    }
  }, [format, options, result]);

  if (!isOpen) return null;

  const accentColor = spaceColor || '#3b82f6';

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
      <div className="bg-[var(--bg-secondary)] border border-[var(--border-color)] rounded-lg shadow-2xl w-[480px] max-w-[90vw]">
        {/* Header */}
        <div 
          className="flex items-center justify-between px-4 py-3 border-b border-[var(--border-color)]"
          style={{ borderTopColor: accentColor }}
        >
          <h2 className="text-lg font-semibold text-[var(--text-primary)]">Export Results</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-[var(--bg-hover)] transition-colors"
          >
            <svg className="w-5 h-5 text-[var(--text-muted)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-4">
          {/* Format Selection */}
          <div>
            <label className="block text-sm font-medium text-[var(--text-primary)] mb-2">Export Format</label>
            <div className="flex gap-2">
              <button
                onClick={() => setFormat('csv')}
                className={`flex-1 px-4 py-2 rounded-lg border-2 transition-all ${
                  format === 'csv'
                    ? 'border-current bg-current/15 text-[var(--text-primary)] font-medium'
                    : 'border-[var(--border-color)] bg-[var(--bg-hover)] text-[var(--text-muted)] hover:bg-[var(--bg-active)] hover:border-[var(--border-subtle)]'
                }`}
                style={format === 'csv' ? { borderColor: accentColor, color: accentColor } : undefined}
              >
                <div className="flex items-center justify-center gap-2">
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                  </svg>
                  <span>CSV</span>
                </div>
              </button>
              <button
                onClick={() => setFormat('json')}
                className={`flex-1 px-4 py-2 rounded-lg border-2 transition-all ${
                  format === 'json'
                    ? 'border-current bg-current/15 text-[var(--text-primary)] font-medium'
                    : 'border-[var(--border-color)] bg-[var(--bg-hover)] text-[var(--text-muted)] hover:bg-[var(--bg-active)] hover:border-[var(--border-subtle)]'
                }`}
                style={format === 'json' ? { borderColor: accentColor, color: accentColor } : undefined}
              >
                <div className="flex items-center justify-center gap-2">
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                  </svg>
                  <span>JSON</span>
                </div>
              </button>
            </div>
          </div>

          {/* Options */}
          <div className="space-y-3">
            <label className="block text-sm font-medium text-[var(--text-primary)]">Options</label>
            
            {format === 'csv' && (
              <>
                <label className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
                  <input
                    type="checkbox"
                    checked={options.include_headers}
                    onChange={(e) => setOptions({ ...options, include_headers: e.target.checked })}
                    className="rounded bg-[var(--bg-hover)] border-[var(--border-color)] text-[var(--accent-color)] focus:ring-[var(--accent-color)]"
                  />
                  Include column headers
                </label>
                
                <div className="flex items-center gap-4">
                  <label className="text-sm text-[var(--text-secondary)]">Delimiter:</label>
                  <select
                    value={options.delimiter}
                    onChange={(e) => setOptions({ ...options, delimiter: e.target.value })}
                    className="bg-[var(--bg-hover)] border border-[var(--border-color)] rounded px-2 py-1 text-sm text-[var(--text-primary)]"
                  >
                    <option value=",">Comma (,)</option>
                    <option value=";">Semicolon (;)</option>
                    <option value="\t">Tab</option>
                    <option value="|">Pipe (|)</option>
                  </select>
                </div>
              </>
            )}

            {format === 'json' && (
              <label className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
                <input
                  type="checkbox"
                  checked={options.pretty_print}
                  onChange={(e) => setOptions({ ...options, pretty_print: e.target.checked })}
                  className="rounded bg-[var(--bg-hover)] border-[var(--border-color)] text-[var(--accent-color)] focus:ring-[var(--accent-color)]"
                />
                Pretty print (formatted output)
              </label>
            )}

            <label className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
              <input
                type="checkbox"
                checked={options.null_as_string}
                onChange={(e) => setOptions({ ...options, null_as_string: e.target.checked })}
                className="rounded bg-[var(--bg-hover)] border-[var(--border-color)] text-[var(--accent-color)] focus:ring-[var(--accent-color)]"
              />
              Show NULL values as "NULL" text
            </label>
          </div>

          {/* Summary */}
          <div className="bg-[var(--bg-hover)] rounded-lg p-3 text-sm">
            <div className="flex justify-between text-[var(--text-muted)]">
              <span>Rows to export:</span>
              <span className="text-[var(--text-primary)]">{result.row_count.toLocaleString()}</span>
            </div>
            <div className="flex justify-between text-[var(--text-muted)]">
              <span>Columns:</span>
              <span className="text-[var(--text-primary)]">{result.columns.length}</span>
            </div>
          </div>

          {/* Progress/Error */}
          {isExporting && (
            <div className="flex items-center gap-3 text-sm text-[var(--text-muted)]">
              <div className="animate-spin w-4 h-4 border-2 border-[var(--accent-color)] border-t-transparent rounded-full" />
              <span>Exporting...</span>
            </div>
          )}

          {progress && progress.is_complete && !error && (
            <div className="flex items-center gap-2 text-sm text-green-400 bg-green-500/10 rounded-lg p-3">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              <span>
                Exported {progress.rows_exported.toLocaleString()} rows 
                ({(progress.bytes_written / 1024).toFixed(1)} KB)
              </span>
            </div>
          )}

          {error && (
            <div className="flex items-center gap-2 text-sm text-red-400 bg-red-500/10 rounded-lg p-3">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <span>{error}</span>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 px-4 py-3 border-t border-[var(--border-color)]">
          <button
            onClick={handleCopyToClipboard}
            disabled={isExporting}
            className="px-4 py-2 rounded-lg border border-[var(--border-color)] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-50"
          >
            Copy to Clipboard
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 rounded-lg border border-[var(--border-color)] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleExport}
            disabled={isExporting}
            className="px-4 py-2 rounded-lg font-medium transition-colors disabled:opacity-50"
            style={{ backgroundColor: accentColor, color: '#ffffff' }}
          >
            {isExporting ? 'Exporting...' : 'Export to File'}
          </button>
        </div>
      </div>
    </div>
  );
}

// Quick export button component for the results grid toolbar
interface ExportButtonProps {
  result: QueryResult;
  spaceColor?: string;
}

export function ExportButton({ result, spaceColor }: ExportButtonProps) {
  const [showDialog, setShowDialog] = useState(false);

  return (
    <>
      <button
        onClick={() => setShowDialog(true)}
        className="flex items-center gap-1.5 px-2.5 py-1.5 rounded text-sm text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
        title="Export results"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
        </svg>
        <span>Export</span>
      </button>

      <ExportDialog
        isOpen={showDialog}
        onClose={() => setShowDialog(false)}
        result={result}
        spaceColor={spaceColor}
      />
    </>
  );
}
