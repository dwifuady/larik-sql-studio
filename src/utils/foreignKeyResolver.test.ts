import { describe, it, expect } from 'vitest';
import {
  buildColumnReferenceIndex,
  findColumnReference,
  resolveColumnSourceTable,
} from './foreignKeyResolver';
import { formatSqlLiteral, qualifiedName, quoteIdentifier } from './sqlLiteral';
import type {
  SchemaColumnInfo,
  SchemaInfo,
  SchemaRelationshipInfo,
  TableInfo,
  VirtualReference,
} from '../types';

function column(name: string, dataType = 'int'): SchemaColumnInfo {
  return {
    name,
    data_type: dataType,
    max_length: null,
    precision: null,
    scale: null,
    is_nullable: true,
    is_primary_key: false,
    is_identity: false,
    is_computed: false,
    column_default: null,
    ordinal_position: 1,
  };
}

function table(name: string, columns: string[], schemaName = 'dbo'): TableInfo {
  return {
    schema_name: schemaName,
    table_name: name,
    table_type: 'BASE TABLE',
    columns: columns.map((c) => column(c)),
  };
}

function relationship(
  constraint: string,
  source: [string, string],
  target: [string, string],
  ordinal = 1
): SchemaRelationshipInfo {
  return {
    constraint_name: constraint,
    ordinal_position: ordinal,
    source_schema_name: 'dbo',
    source_table_name: source[0],
    source_column_name: source[1],
    target_schema_name: 'dbo',
    target_table_name: target[0],
    target_column_name: target[1],
  };
}

const schemaInfo: SchemaInfo = {
  database_name: 'AppDb',
  schemas: ['dbo'],
  tables: [
    table('Transaction', ['Id', 'Status', 'UserId', 'TenantId', 'Amount']),
    table('TransactionStatus', ['Code', 'Name']),
    table('User', ['Id', 'TenantId', 'Email']),
    table('Audit', ['Id', 'Status']),
  ],
  relationships: [
    relationship('FK_Transaction_Status', ['Transaction', 'Status'], ['TransactionStatus', 'Code']),
    relationship('FK_Transaction_User', ['Transaction', 'UserId'], ['User', 'Id'], 1),
    relationship('FK_Transaction_User', ['Transaction', 'TenantId'], ['User', 'TenantId'], 2),
  ],
  routines: [],
  fetched_at: '2026-07-30T00:00:00Z',
};

describe('resolveColumnSourceTable', () => {
  it('resolves against a single-table query', () => {
    expect(resolveColumnSourceTable('SELECT * FROM dbo.[Transaction]', 'Status', schemaInfo)).toEqual({
      schema: 'dbo',
      table: 'Transaction',
    });
  });

  it('resolves an unambiguous column in a join', () => {
    const sql = 'SELECT t.Status, u.Email FROM dbo.[Transaction] t JOIN dbo.[User] u ON u.Id = t.UserId';
    expect(resolveColumnSourceTable(sql, 'Status', schemaInfo)).toEqual({ schema: 'dbo', table: 'Transaction' });
    expect(resolveColumnSourceTable(sql, 'Email', schemaInfo)).toEqual({ schema: 'dbo', table: 'User' });
  });

  it('returns null when a column name exists in more than one joined table', () => {
    const sql = 'SELECT * FROM dbo.[Transaction] t JOIN dbo.Audit a ON a.Id = t.Id';
    expect(resolveColumnSourceTable(sql, 'Status', schemaInfo)).toBeNull();
  });

  it('returns null for unknown tables and empty input', () => {
    expect(resolveColumnSourceTable('SELECT * FROM dbo.Missing', 'Status', schemaInfo)).toBeNull();
    expect(resolveColumnSourceTable('', 'Status', schemaInfo)).toBeNull();
    expect(resolveColumnSourceTable('SELECT * FROM dbo.[Transaction]', 'Status', null)).toBeNull();
  });
});

describe('findColumnReference', () => {
  it('finds a single column foreign key', () => {
    const reference = findColumnReference('SELECT * FROM dbo.[Transaction]', 'Status', schemaInfo);
    expect(reference).not.toBeNull();
    expect(reference!.targetTable).toBe('TransactionStatus');
    expect(reference!.targetColumn).toBe('Code');
    expect(reference!.columnPairs).toEqual([{ sourceColumn: 'Status', targetColumn: 'Code' }]);
  });

  it('returns every column pair of a composite foreign key', () => {
    const reference = findColumnReference('SELECT * FROM dbo.[Transaction]', 'UserId', schemaInfo);
    expect(reference).not.toBeNull();
    expect(reference!.constraintName).toBe('FK_Transaction_User');
    expect(reference!.targetColumn).toBe('Id');
    expect(reference!.columnPairs).toEqual([
      { sourceColumn: 'UserId', targetColumn: 'Id' },
      { sourceColumn: 'TenantId', targetColumn: 'TenantId' },
    ]);
  });

  it('returns null for columns without a foreign key', () => {
    expect(findColumnReference('SELECT * FROM dbo.[Transaction]', 'Amount', schemaInfo)).toBeNull();
  });
});

describe('buildColumnReferenceIndex', () => {
  it('maps only the columns that reference another table, but sources them all', () => {
    const { sources, references } = buildColumnReferenceIndex(
      'SELECT Id, Status, Amount FROM dbo.[Transaction]',
      [{ name: 'Id' }, { name: 'Status' }, { name: 'Amount' }],
      schemaInfo
    );
    expect(Array.from(references.keys())).toEqual([1]);
    expect(references.get(1)!.targetTable).toBe('TransactionStatus');
    expect(references.get(1)!.isVirtual).toBe(false);
    // Every traceable column has a source so the UI can offer a custom reference
    expect(Array.from(sources.keys())).toEqual([0, 1, 2]);
    expect(sources.get(2)).toEqual({ schema: 'dbo', table: 'Transaction' });
  });

  it('is empty when there is no schema metadata', () => {
    const { sources, references } = buildColumnReferenceIndex(
      'SELECT Status FROM dbo.[Transaction]',
      [{ name: 'Status' }],
      null
    );
    expect(references.size).toBe(0);
    expect(sources.size).toBe(0);
  });
});

describe('virtual (user-defined) references', () => {
  const virtualReference = (
    sourceColumn: string,
    targetTable: string,
    targetColumn: string
  ): VirtualReference => ({
    id: `v-${sourceColumn}`,
    connection_id: 'space1',
    database_name: 'AppDb',
    source_schema: 'dbo',
    source_table: 'Transaction',
    source_column: sourceColumn,
    target_schema: 'dbo',
    target_table: targetTable,
    target_column: targetColumn,
    created_at: '2026-07-30T00:00:00Z',
    updated_at: '2026-07-30T00:00:00Z',
  });

  it('resolves a column that has no database foreign key', () => {
    const reference = findColumnReference('SELECT * FROM dbo.[Transaction]', 'Amount', schemaInfo, [
      virtualReference('Amount', 'TransactionStatus', 'Code'),
    ]);
    expect(reference).not.toBeNull();
    expect(reference!.isVirtual).toBe(true);
    expect(reference!.constraintName).toBeNull();
    expect(reference!.virtualReferenceId).toBe('v-Amount');
    expect(reference!.columnPairs).toEqual([{ sourceColumn: 'Amount', targetColumn: 'Code' }]);
  });

  it('matches source identifiers case-insensitively', () => {
    const reference = findColumnReference('SELECT * FROM dbo.[Transaction]', 'Amount', schemaInfo, [
      virtualReference('amount', 'TransactionStatus', 'Code'),
    ]);
    expect(reference?.isVirtual).toBe(true);
  });

  it('lets a real foreign key win over a user-defined one on the same column', () => {
    const reference = findColumnReference('SELECT * FROM dbo.[Transaction]', 'Status', schemaInfo, [
      virtualReference('Status', 'User', 'Id'),
    ]);
    expect(reference!.isVirtual).toBe(false);
    expect(reference!.targetTable).toBe('TransactionStatus');
    expect(reference!.constraintName).toBe('FK_Transaction_Status');
  });

  it('ignores references defined for another table', () => {
    const other: VirtualReference = { ...virtualReference('Amount', 'User', 'Id'), source_table: 'Audit' };
    expect(findColumnReference('SELECT * FROM dbo.[Transaction]', 'Amount', schemaInfo, [other])).toBeNull();
  });

  it('surfaces user-defined references through the column index', () => {
    const { references } = buildColumnReferenceIndex(
      'SELECT Id, Amount FROM dbo.[Transaction]',
      [{ name: 'Id' }, { name: 'Amount' }],
      schemaInfo,
      [virtualReference('Amount', 'TransactionStatus', 'Code')]
    );
    expect(references.get(1)!.isVirtual).toBe(true);
    expect(references.get(1)!.targetTable).toBe('TransactionStatus');
  });
});

describe('sqlLiteral', () => {
  it('quotes identifiers and escapes brackets', () => {
    expect(quoteIdentifier('Status')).toBe('[Status]');
    expect(quoteIdentifier('od]d')).toBe('[od]]d]');
    expect(qualifiedName('dbo', 'Transaction')).toBe('[dbo].[Transaction]');
  });

  it('formats numbers without quotes', () => {
    expect(formatSqlLiteral(5, 'int')).toBe('5');
    expect(formatSqlLiteral('42', 'int')).toBe('42');
  });

  it('escapes embedded single quotes', () => {
    expect(formatSqlLiteral("O'Brien", 'varchar')).toBe("'O''Brien'");
  });

  it('uses the N prefix only for unicode types', () => {
    expect(formatSqlLiteral('PAID', 'nvarchar')).toBe("N'PAID'");
    expect(formatSqlLiteral('PAID', 'varchar')).toBe("'PAID'");
  });

  it('maps bit values to 1/0 and rejects NULL', () => {
    expect(formatSqlLiteral('true', 'bit')).toBe('1');
    expect(formatSqlLiteral(false, 'bit')).toBe('0');
    expect(formatSqlLiteral(null, 'int')).toBeNull();
  });

  it('renders binary values as hex', () => {
    expect(formatSqlLiteral([0, 15, 255], 'varbinary')).toBe('0x000fff');
  });
});
