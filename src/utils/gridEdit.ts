// SQL builders for inline grid editing (UPDATE / INSERT / DELETE).
// Shared by ResultsGrid so add, edit, and delete all produce consistent literals.
import type { CellValue } from '../types';

/** Minimal column reference needed to build SQL for one column. */
export interface GridColumnRef {
  name: string;
  dataType: string;
}

/** Format a cell value as a SQL literal (NULL, number, hex binary, or quoted string). */
export function formatValueForInsert(value: CellValue, dataType: string): string {
  if (value === null) {
    return 'NULL';
  }
  if (typeof value === 'boolean') {
    return value ? '1' : '0';
  }
  if (typeof value === 'number') {
    return String(value);
  }
  if (Array.isArray(value)) {
    // Binary data - convert to hex string
    const hex = value.map(b => b.toString(16).padStart(2, '0')).join('');
    return `0x${hex}`;
  }
  // String value - escape single quotes
  const type = dataType.toLowerCase();
  const strValue = String(value).replace(/'/g, "''");

  // For date/time types, use appropriate format
  if (type.includes('date') || type.includes('time')) {
    return `'${strValue}'`;
  }

  // For numeric types stored as string, don't quote
  if ((type.includes('int') || type.includes('numeric') || type.includes('decimal') ||
    type.includes('float') || type.includes('real') || type.includes('money')) &&
    !isNaN(Number(strValue))) {
    return strValue;
  }

  // String - wrap with N' for nvarchar/nchar support
  if (type.includes('nvarchar') || type.includes('nchar') || type.includes('ntext')) {
    return `N'${strValue}'`;
  }

  return `'${strValue}'`;
}

/**
 * Coerce a raw string from the inline editor into a CellValue based on the
 * target column's data type. Used for newly added rows where no original
 * value exists to infer the type from.
 */
export function coerceCellValue(raw: string, dataType: string): CellValue {
  if (raw === '' || raw.toLowerCase() === 'null') {
    return null;
  }
  const type = dataType.toLowerCase();
  if (
    type.includes('int') || type.includes('numeric') || type.includes('decimal') ||
    type.includes('float') || type.includes('real') || type.includes('money')
  ) {
    const num = Number(raw);
    if (!isNaN(num)) return num;
    return raw;
  }
  if (type.includes('bit')) {
    return raw === '1' || raw.toLowerCase() === 'true';
  }
  return raw;
}

/** Build an UPDATE statement setting each column to its new value. */
export function buildUpdateQuery(
  tableName: string,
  identity: GridColumnRef,
  identityValue: CellValue,
  sets: Array<{ column: GridColumnRef; value: CellValue }>
): string {
  const setClauses = sets.map(({ column, value }) =>
    `[${column.name}] = ${formatValueForInsert(value, column.dataType)}`
  );
  const identityFormatted = formatValueForInsert(identityValue, identity.dataType);
  return `UPDATE ${tableName} SET ${setClauses.join(', ')} WHERE [${identity.name}] = ${identityFormatted}`;
}

/** Build a DELETE statement targeting one row by its key column. */
export function buildDeleteQuery(
  tableName: string,
  identity: GridColumnRef,
  identityValue: CellValue
): string {
  const identityFormatted = formatValueForInsert(identityValue, identity.dataType);
  return `DELETE FROM ${tableName} WHERE [${identity.name}] = ${identityFormatted}`;
}

/** Build an INSERT statement for explicitly provided columns only. */
export function buildInsertQuery(
  tableName: string,
  columns: GridColumnRef[],
  values: CellValue[]
): string {
  const colNames = columns.map(c => `[${c.name}]`).join(', ');
  const formattedValues = columns
    .map((c, i) => formatValueForInsert(values[i] ?? null, c.dataType))
    .join(', ');
  return `INSERT INTO ${tableName} (${colNames}) VALUES (${formattedValues})`;
}
