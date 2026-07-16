import { describe, it, expect } from 'vitest';
import type { SchemaInfo } from '../types';
import { buildJoinConditionSuggestions, getJoinSuggestionDebugRefs, isJoinConditionContext } from './sqlJoinSuggestions';

const baseSchema: SchemaInfo = {
  database_name: 'testdb',
  schemas: ['dbo'],
  tables: [
    {
      schema_name: 'dbo',
      table_name: 'Application',
      table_type: 'BASE TABLE',
      columns: [
        { name: 'Id', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: true, is_identity: true, column_default: null, ordinal_position: 1 },
        { name: 'Name', data_type: 'nvarchar', max_length: 100, precision: null, scale: null, is_nullable: false, is_primary_key: false, is_identity: false, column_default: null, ordinal_position: 2 },
      ],
    },
    {
      schema_name: 'dbo',
      table_name: 'BCApplicationQuotes',
      table_type: 'BASE TABLE',
      columns: [
        { name: 'Id', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: true, is_identity: true, column_default: null, ordinal_position: 1 },
        { name: 'ApplicationId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: false, is_identity: false, column_default: null, ordinal_position: 2 },
      ],
    },
    {
      schema_name: 'dbo',
      table_name: 'ApplicationAudit',
      table_type: 'BASE TABLE',
      columns: [
        { name: 'Id', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: true, is_identity: true, column_default: null, ordinal_position: 1 },
        { name: 'ApplicationId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: false, is_identity: false, column_default: null, ordinal_position: 2 },
      ],
    },
    {
      schema_name: 'dbo',
      table_name: 'TenantUser',
      table_type: 'BASE TABLE',
      columns: [
        { name: 'TenantId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: true, is_identity: false, column_default: null, ordinal_position: 1 },
        { name: 'UserId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: true, is_identity: false, column_default: null, ordinal_position: 2 },
      ],
    },
    {
      schema_name: 'dbo',
      table_name: 'TenantPermission',
      table_type: 'BASE TABLE',
      columns: [
        { name: 'TenantId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: false, is_identity: false, column_default: null, ordinal_position: 1 },
        { name: 'UserId', data_type: 'int', max_length: null, precision: null, scale: null, is_nullable: false, is_primary_key: false, is_identity: false, column_default: null, ordinal_position: 2 },
      ],
    },
  ],
  relationships: [],
  routines: [],
  fetched_at: '2026-04-09T00:00:00Z',
};

describe('sqlJoinSuggestions', () => {
  it('detects join-condition context after ON', () => {
    const sql = `
      SELECT *
      FROM [dbo].[Application] AS [a]
      JOIN [dbo].[BCApplicationQuotes] AS [bcaq] ON 
    `.trimEnd();

    expect(isJoinConditionContext(sql)).toBe(true);
  });

  it('extracts ordered refs from an incomplete join', () => {
    const sql = `
      SELECT *
      FROM [dbo].[Application] AS [a]
      JOIN [dbo].[BCApplicationQuotes] AS [bcaq] ON 
    `.trimEnd();

    expect(getJoinSuggestionDebugRefs(sql)).toEqual([
      { schema: 'dbo', table: 'Application', alias: 'a' },
      { schema: 'dbo', table: 'BCApplicationQuotes', alias: 'bcaq' },
    ]);
  });

  it('suggests heuristic join predicates when no foreign key metadata exists', () => {
    const sql = `
      SELECT *
      FROM [dbo].[Application] AS [a]
      JOIN [dbo].[BCApplicationQuotes] AS [bcaq] ON 
    `.trimEnd();

    const suggestions = buildJoinConditionSuggestions(sql, baseSchema);

    expect(suggestions[0]?.detail).toBe('Heuristic match');
    expect(suggestions[0]?.insertText).toBe('[bcaq].[ApplicationId] = [a].[Id]');
  });

  it('prefers foreign key suggestions over heuristics', () => {
    const schema: SchemaInfo = {
      ...baseSchema,
      relationships: [
        {
          constraint_name: 'FK_BCApplicationQuotes_Application',
          ordinal_position: 1,
          source_schema_name: 'dbo',
          source_table_name: 'BCApplicationQuotes',
          source_column_name: 'ApplicationId',
          target_schema_name: 'dbo',
          target_table_name: 'Application',
          target_column_name: 'Id',
        },
      ],
    };

    const sql = `
      SELECT *
      FROM [dbo].[Application] AS [a]
      JOIN [dbo].[BCApplicationQuotes] AS [bcaq] ON 
    `.trimEnd();

    const suggestions = buildJoinConditionSuggestions(sql, schema);

    expect(suggestions[0]?.detail).toBe('Foreign key');
    expect(suggestions[0]?.insertText).toBe('[bcaq].[ApplicationId] = [a].[Id]');
  });

  it('builds composite join predicates from multi-column foreign keys', () => {
    const schema: SchemaInfo = {
      ...baseSchema,
      relationships: [
        {
          constraint_name: 'FK_TenantPermission_TenantUser',
          ordinal_position: 1,
          source_schema_name: 'dbo',
          source_table_name: 'TenantPermission',
          source_column_name: 'TenantId',
          target_schema_name: 'dbo',
          target_table_name: 'TenantUser',
          target_column_name: 'TenantId',
        },
        {
          constraint_name: 'FK_TenantPermission_TenantUser',
          ordinal_position: 2,
          source_schema_name: 'dbo',
          source_table_name: 'TenantPermission',
          source_column_name: 'UserId',
          target_schema_name: 'dbo',
          target_table_name: 'TenantUser',
          target_column_name: 'UserId',
        },
      ],
    };

    const sql = `
      SELECT *
      FROM [dbo].[TenantUser] AS [tu]
      JOIN [dbo].[TenantPermission] AS [tp] ON 
    `.trimEnd();

    const suggestions = buildJoinConditionSuggestions(sql, schema);

    expect(suggestions[0]?.insertText).toBe('[tp].[TenantId] = [tu].[TenantId] AND [tp].[UserId] = [tu].[UserId]');
  });
});
