/**
 * React hook for SQL validation in Monaco editor
 * Provides real-time SQL syntax and schema validation
 */

import { useEffect, useRef } from 'react';
import type { editor } from 'monaco-editor';
import { SqlValidator } from '../utils/sqlValidator';
import type { SchemaInfo } from '../types';

export interface UseSqlValidationOptions {
  /** Monaco editor instance */
  editor: editor.IStandaloneCodeEditor | null;
  /** Monaco module (for markers API) */
  monaco: typeof import('monaco-editor') | null;
  /** Schema information for semantic validation */
  schemaInfo: SchemaInfo | null;
  /** Whether validation is enabled */
  enabled: boolean;
  /** Show warning-level messages */
  showWarnings?: boolean;
  /** Show info-level messages */
  showInfo?: boolean;
  /** Debounce time in milliseconds (default: 500ms) */
  debounceMs?: number;
}

/**
 * Hook to add SQL validation to Monaco editor
 * Validates syntax and schema references with debouncing
 */
export function useSqlValidation({
  editor,
  monaco,
  schemaInfo,
  enabled,
  showWarnings = true,
  showInfo = true,
  debounceMs = 500,
}: UseSqlValidationOptions): void {
  const validatorRef = useRef<SqlValidator | undefined>(undefined);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    // Initialize validator once
    if (!validatorRef.current) {
      validatorRef.current = new SqlValidator();
    }

    // Don't set up validation if editor/monaco not ready or validation disabled
    if (!editor || !monaco || !enabled) {
      // Clear any existing markers if validation is disabled
      if (editor && monaco) {
        const model = editor.getModel();
        if (model) {
          monaco.editor.setModelMarkers(model, 'sql-validator', []);
        }
      }
      return;
    }

    const model = editor.getModel();
    if (!model) {
      return;
    }

    /**
     * Perform validation on current editor content
     */
    const validate = () => {
      const currentModel = editor.getModel();
      if (!currentModel) return;

      const query = currentModel.getValue();
      const validator = validatorRef.current!;

      try {
        const errors = validator.validateQuery(query, schemaInfo);

        // Filter errors based on severity settings
        const filteredErrors = errors.filter((err) => {
          if (err.severity === 'error') return true;
          if (err.severity === 'warning' && !showWarnings) return false;
          if (err.severity === 'info' && !showInfo) return false;
          return true;
        });

        // Convert ValidationError[] to Monaco markers
        const markers = filteredErrors.map((err) => {
          // Map severity (MarkerSeverity enum: Hint=1, Info=2, Warning=4, Error=8)
          let severity: number;
          if (err.severity === 'error') {
            severity = 8; // MarkerSeverity.Error
          } else if (err.severity === 'warning') {
            severity = 4; // MarkerSeverity.Warning
          } else {
            severity = 2; // MarkerSeverity.Info
          }

          return {
            severity,
            startLineNumber: err.line,
            startColumn: err.column,
            endLineNumber: err.endLine,
            endColumn: err.endColumn,
            message: err.message,
            code: err.code,
          };
        });

        // Set markers on the model
        monaco.editor.setModelMarkers(currentModel, 'sql-validator', markers);
      } catch (err) {
        // Validation failed - log error but don't break UI
        console.error('SQL validation error:', err);
        // Clear markers on validation failure
        monaco.editor.setModelMarkers(currentModel, 'sql-validator', []);
      }
    };

    /**
     * Debounced validation on content change
     */
    const disposable = editor.onDidChangeModelContent(() => {
      // Clear previous timeout
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }

      // Set new timeout for validation
      timeoutRef.current = setTimeout(validate, debounceMs);
    });

    // Perform initial validation
    validate();

    // Cleanup function
    return () => {
      // Dispose content change listener
      disposable.dispose();

      // Clear pending timeout
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }

      // Clear markers on unmount
      const currentModel = editor.getModel();
      if (currentModel && monaco) {
        monaco.editor.setModelMarkers(currentModel, 'sql-validator', []);
      }
    };
  }, [editor, monaco, schemaInfo, enabled, showWarnings, showInfo, debounceMs]);
}
