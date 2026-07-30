import type { CellValue } from '../types';

/** Quote an identifier for T-SQL, escaping any embedded closing bracket. */
export function quoteIdentifier(name: string): string {
  return `[${name.replace(/]/g, ']]')}]`;
}

/** Build a `[schema].[table]` name. */
export function qualifiedName(schemaName: string, tableName: string): string {
  return `${quoteIdentifier(schemaName)}.${quoteIdentifier(tableName)}`;
}

function escapeStringLiteral(value: string): string {
  return value.replace(/'/g, "''");
}

function isNumericType(type: string): boolean {
  return (
    type.includes('int') ||
    type.includes('numeric') ||
    type.includes('decimal') ||
    type.includes('float') ||
    type.includes('real') ||
    type.includes('money')
  );
}

/** Unicode string types need the N'' prefix. Non-unicode ones must NOT get it:
 *  comparing a varchar column to an nvarchar literal forces an implicit
 *  conversion of the column and turns an index seek into a scan. */
function isUnicodeType(type: string): boolean {
  return type.includes('nvarchar') || type.includes('nchar') || type.includes('ntext');
}

function toHexLiteral(bytes: number[]): string {
  const hex = bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0')).join('');
  return `0x${hex}`;
}

/**
 * Format a grid cell value as a T-SQL literal suitable for a WHERE comparison.
 *
 * Returns `null` when the value cannot be expressed safely (NULL, NaN, or a
 * type we don't want to guess at) — callers should skip the predicate rather
 * than emit something that could match the wrong row.
 */
export function formatSqlLiteral(value: CellValue, dataType: string): string | null {
  if (value === null || value === undefined) return null;

  const type = (dataType || '').toLowerCase();

  if (Array.isArray(value)) {
    return value.length > 0 ? toHexLiteral(value) : null;
  }

  if (typeof value === 'boolean') {
    return value ? '1' : '0';
  }

  if (typeof value === 'number') {
    return Number.isFinite(value) ? String(value) : null;
  }

  const text = String(value);

  if (type.includes('bit') || type.includes('bool')) {
    const lower = text.trim().toLowerCase();
    if (lower === 'true' || lower === '1') return '1';
    if (lower === 'false' || lower === '0') return '0';
    return null;
  }

  if (isNumericType(type)) {
    const trimmed = text.trim();
    // Only pass through when it really is a number; otherwise quote it so a
    // malformed value produces "no match" instead of a syntax error.
    if (trimmed !== '' && !Number.isNaN(Number(trimmed))) {
      return trimmed;
    }
    return `'${escapeStringLiteral(trimmed)}'`;
  }

  if (isUnicodeType(type)) {
    return `N'${escapeStringLiteral(text)}'`;
  }

  return `'${escapeStringLiteral(text)}'`;
}
