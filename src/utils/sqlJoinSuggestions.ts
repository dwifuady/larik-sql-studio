import type { SchemaInfo, SchemaRelationshipInfo, TableInfo, SchemaColumnInfo } from '../types';

export interface JoinConditionSuggestion {
  label: string;
  detail: string;
  documentation: string;
  insertText: string;
  score: number;
}

interface TableRef {
  schema: string;
  table: string;
  alias: string;
}

interface RelationshipMatch {
  relationship: SchemaRelationshipInfo;
  otherAlias: string;
  otherSchema: string;
  otherTable: string;
  joinedColumn: string;
  otherColumn: string;
  isReverse: boolean;
}

function stripIdentifier(identifier: string | undefined): string {
  return (identifier || '').replace(/[\[\]]/g, '').trim();
}

function normalizeName(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, '');
}

function singularize(value: string): string {
  if (value.endsWith('ies')) return `${value.slice(0, -3)}y`;
  if (value.endsWith('ses')) return value.slice(0, -2);
  if (value.endsWith('s') && !value.endsWith('ss')) return value.slice(0, -1);
  return value;
}

function getTableVariants(tableName: string): Set<string> {
  const normalized = normalizeName(tableName);
  const singular = singularize(normalized);
  return new Set([normalized, singular].filter(Boolean));
}

function columnStem(columnName: string): string | null {
  const normalized = normalizeName(columnName);
  if (!normalized.endsWith('id') || normalized.length <= 2) {
    return null;
  }

  return normalized.slice(0, -2);
}

function tableKey(schemaName: string, tableName: string): string {
  return `${schemaName.toLowerCase()}.${tableName.toLowerCase()}`;
}

function findTable(schema: SchemaInfo, schemaName: string, tableName: string): TableInfo | undefined {
  return schema.tables.find(table =>
    table.schema_name.toLowerCase() === schemaName.toLowerCase()
    && table.table_name.toLowerCase() === tableName.toLowerCase()
  );
}

function quoteIdentifier(identifier: string): string {
  return `[${identifier}]`;
}

function formatColumn(alias: string, column: string): string {
  return `${quoteIdentifier(alias)}.${quoteIdentifier(column)}`;
}

function extractOrderedTableRefs(sql: string): TableRef[] {
  const refs: TableRef[] = [];
  const tablePattern = /\b(?:FROM|INNER\s+JOIN|LEFT\s+JOIN|RIGHT\s+JOIN|FULL\s+JOIN|CROSS\s+JOIN|OUTER\s+JOIN|JOIN|CROSS\s+APPLY|OUTER\s+APPLY|APPLY)\s+((?:\[[^\]]+\]|\w+))(?:\s*\.\s*((?:\[[^\]]+\]|\w+)))?(?:\s+(?:AS\s+)?((?:\[[^\]]+\]|\w+)))?/gi;
  const skipWords = new Set(['ON', 'WHERE', 'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER', 'FULL', 'CROSS', 'ORDER', 'GROUP', 'HAVING', 'UNION', 'APPLY']);

  for (const match of sql.matchAll(tablePattern)) {
    const first = stripIdentifier(match[1]);
    const second = stripIdentifier(match[2]);
    let alias = stripIdentifier(match[3]);

    let schema = 'dbo';
    let table = first;

    if (second) {
      schema = first;
      table = second;
    }

    if (!table) {
      continue;
    }

    if (!alias || skipWords.has(alias.toUpperCase())) {
      alias = table;
    }

    refs.push({ schema, table, alias });
  }

  return refs;
}

export function isJoinConditionContext(textBeforeCursor: string): boolean {
  return /\bON\b\s*(?:(?:\[(?:\w*)\]?|\w+)\s*)?$/i.test(textBeforeCursor);
}

function buildRelationshipSuggestions(schema: SchemaInfo, joinedRef: TableRef, priorRefs: TableRef[]): JoinConditionSuggestion[] {
  const grouped = new Map<string, { score: number; kind: string; otherAlias: string; pairs: Array<{ joinedColumn: string; otherColumn: string; ordinal: number }> }>();

  schema.relationships.forEach(relationship => {
    priorRefs.forEach(otherRef => {
      let match: RelationshipMatch | null = null;

      if (
        relationship.source_schema_name.toLowerCase() === joinedRef.schema.toLowerCase()
        && relationship.source_table_name.toLowerCase() === joinedRef.table.toLowerCase()
        && relationship.target_schema_name.toLowerCase() === otherRef.schema.toLowerCase()
        && relationship.target_table_name.toLowerCase() === otherRef.table.toLowerCase()
      ) {
        match = {
          relationship,
          otherAlias: otherRef.alias,
          otherSchema: otherRef.schema,
          otherTable: otherRef.table,
          joinedColumn: relationship.source_column_name,
          otherColumn: relationship.target_column_name,
          isReverse: false,
        };
      } else if (
        relationship.target_schema_name.toLowerCase() === joinedRef.schema.toLowerCase()
        && relationship.target_table_name.toLowerCase() === joinedRef.table.toLowerCase()
        && relationship.source_schema_name.toLowerCase() === otherRef.schema.toLowerCase()
        && relationship.source_table_name.toLowerCase() === otherRef.table.toLowerCase()
      ) {
        match = {
          relationship,
          otherAlias: otherRef.alias,
          otherSchema: otherRef.schema,
          otherTable: otherRef.table,
          joinedColumn: relationship.target_column_name,
          otherColumn: relationship.source_column_name,
          isReverse: true,
        };
      }

      if (!match) return;

      const key = `${relationship.constraint_name.toLowerCase()}::${otherRef.alias.toLowerCase()}::${match.isReverse ? 'reverse' : 'forward'}`;
      if (!grouped.has(key)) {
        grouped.set(key, {
          score: match.isReverse ? 980 : 1000,
          kind: match.isReverse ? 'Foreign key (reverse)' : 'Foreign key',
          otherAlias: otherRef.alias,
          pairs: [],
        });
      }

      grouped.get(key)!.pairs.push({
        joinedColumn: match.joinedColumn,
        otherColumn: match.otherColumn,
        ordinal: relationship.ordinal_position,
      });
    });
  });

  return Array.from(grouped.values()).map(group => {
    const orderedPairs = group.pairs.sort((a, b) => a.ordinal - b.ordinal);
    const insertText = orderedPairs
      .map(pair => `${formatColumn(joinedRef.alias, pair.joinedColumn)} = ${formatColumn(group.otherAlias, pair.otherColumn)}`)
      .join(' AND ');

    const preview = orderedPairs[0]
      ? `${orderedPairs[0].joinedColumn} = ${orderedPairs[0].otherColumn}`
      : 'Join columns';

    return {
      label: `Join on ${preview}`,
      detail: group.kind,
      documentation: `Suggested join between ${quoteIdentifier(joinedRef.alias)} and ${quoteIdentifier(group.otherAlias)}`,
      insertText,
      score: group.score - Math.max(0, orderedPairs.length - 1),
    };
  });
}

function getPreferredKeyColumns(table: TableInfo): SchemaColumnInfo[] {
  const primaryKeys = table.columns.filter(column => column.is_primary_key);
  if (primaryKeys.length > 0) {
    return primaryKeys;
  }

  const idColumn = table.columns.find(column => normalizeName(column.name) === 'id');
  return idColumn ? [idColumn] : [];
}

function matchesTableStem(stem: string, tableName: string): boolean {
  const variants = getTableVariants(tableName);
  return Array.from(variants).some(variant => stem === variant || stem.endsWith(variant) || variant.endsWith(stem));
}

function pushSuggestion(
  suggestions: Map<string, JoinConditionSuggestion>,
  suggestion: JoinConditionSuggestion
): void {
  const existing = suggestions.get(suggestion.insertText);
  if (!existing || suggestion.score > existing.score) {
    suggestions.set(suggestion.insertText, suggestion);
  }
}

function buildHeuristicSuggestions(schema: SchemaInfo, joinedRef: TableRef, priorRefs: TableRef[]): JoinConditionSuggestion[] {
  const joinedTable = findTable(schema, joinedRef.schema, joinedRef.table);
  if (!joinedTable) {
    return [];
  }

  const suggestions = new Map<string, JoinConditionSuggestion>();

  priorRefs.forEach(otherRef => {
    const otherTable = findTable(schema, otherRef.schema, otherRef.table);
    if (!otherTable) {
      return;
    }

    const otherKeyColumns = getPreferredKeyColumns(otherTable);
    const joinedKeyColumns = getPreferredKeyColumns(joinedTable);

    joinedTable.columns.forEach(joinedColumn => {
      const joinedStem = columnStem(joinedColumn.name);
      if (joinedStem && otherKeyColumns.length > 0 && matchesTableStem(joinedStem, otherTable.table_name)) {
        otherKeyColumns.forEach(otherKeyColumn => {
          pushSuggestion(suggestions, {
            label: `Join on ${joinedColumn.name} = ${otherKeyColumn.name}`,
            detail: 'Heuristic match',
            documentation: `Matched ${joinedColumn.name} to ${otherTable.table_name}.${otherKeyColumn.name} using table-name heuristic.`,
            insertText: `${formatColumn(joinedRef.alias, joinedColumn.name)} = ${formatColumn(otherRef.alias, otherKeyColumn.name)}`,
            score: normalizeName(otherKeyColumn.name) === 'id' ? 900 : 875,
          });
        });
      }

      otherTable.columns.forEach(otherColumn => {
        const joinedNormalized = normalizeName(joinedColumn.name);
        const otherNormalized = normalizeName(otherColumn.name);
        if (joinedNormalized === otherNormalized) {
          pushSuggestion(suggestions, {
            label: `Join on ${joinedColumn.name} = ${otherColumn.name}`,
            detail: 'Heuristic match',
            documentation: `Matched identical column names across ${joinedTable.table_name} and ${otherTable.table_name}.`,
            insertText: `${formatColumn(joinedRef.alias, joinedColumn.name)} = ${formatColumn(otherRef.alias, otherColumn.name)}`,
            score: joinedColumn.is_primary_key || otherColumn.is_primary_key ? 760 : 720,
          });
        }
      });
    });

    otherTable.columns.forEach(otherColumn => {
      const otherStem = columnStem(otherColumn.name);
      if (otherStem && joinedKeyColumns.length > 0 && matchesTableStem(otherStem, joinedTable.table_name)) {
        joinedKeyColumns.forEach(joinedKeyColumn => {
          pushSuggestion(suggestions, {
            label: `Join on ${joinedKeyColumn.name} = ${otherColumn.name}`,
            detail: 'Heuristic match (reverse)',
            documentation: `Matched ${otherColumn.name} back to ${joinedTable.table_name}.${joinedKeyColumn.name} using table-name heuristic.`,
            insertText: `${formatColumn(joinedRef.alias, joinedKeyColumn.name)} = ${formatColumn(otherRef.alias, otherColumn.name)}`,
            score: normalizeName(joinedKeyColumn.name) === 'id' ? 840 : 815,
          });
        });
      }
    });
  });

  return Array.from(suggestions.values());
}

export function buildJoinConditionSuggestions(
  textBeforeCursor: string,
  schema: SchemaInfo | null
): JoinConditionSuggestion[] {
  if (!schema || !isJoinConditionContext(textBeforeCursor)) {
    return [];
  }

  const orderedRefs = extractOrderedTableRefs(textBeforeCursor);
  if (orderedRefs.length < 2) {
    return [];
  }

  const joinedRef = orderedRefs[orderedRefs.length - 1];
  const priorRefs = orderedRefs.slice(0, -1)
    .filter((ref, index, refs) => refs.findIndex(candidate => candidate.alias.toLowerCase() === ref.alias.toLowerCase()) === index)
    .reverse();

  if (!findTable(schema, joinedRef.schema, joinedRef.table)) {
    return [];
  }

  const allSuggestions = [
    ...buildRelationshipSuggestions(schema, joinedRef, priorRefs),
    ...buildHeuristicSuggestions(schema, joinedRef, priorRefs),
  ];

  const deduped = new Map<string, JoinConditionSuggestion>();
  allSuggestions.forEach(suggestion => pushSuggestion(deduped, suggestion));

  return Array.from(deduped.values()).sort((a, b) => b.score - a.score || a.label.localeCompare(b.label));
}

export function getJoinSuggestionDebugRefs(textBeforeCursor: string): TableRef[] {
  return extractOrderedTableRefs(textBeforeCursor);
}

export function getSchemaTableKey(schemaName: string, tableName: string): string {
  return tableKey(schemaName, tableName);
}
