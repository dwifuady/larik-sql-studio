// DBeaver Templates XML Parser (T046)
// Parses DBeaver's templates.xml format and converts to our snippet format

import type { CreateSnippetInput, DbeaverSnippet } from '../types';

/**
 * Parse DBeaver templates.xml content into snippets
 * 
 * DBeaver format:
 * <templates>
 *   <template autoinsert="true" context="sql" deleted="false" description="" enabled="true" name="sel">
 *     SELECT * FROM ${cursor}
 *   </template>
 * </templates>
 * 
 * The name attribute is the trigger, content is the body.
 * DBeaver uses ${cursor} for cursor position (same as us).
 * Variable placeholders like $table_name$ need to be converted.
 */
export function parseDbeaverTemplatesXml(xmlContent: string): DbeaverSnippet[] {
  const snippets: DbeaverSnippet[] = [];

  try {
    // Parse XML using DOMParser (available in browser)
    const parser = new DOMParser();
    const doc = parser.parseFromString(xmlContent, 'application/xml');

    // Check for parse errors
    const parserError = doc.querySelector('parsererror');
    if (parserError) {
      console.error('XML parse error:', parserError.textContent);
      return [];
    }

    // Find all template elements
    const templates = doc.querySelectorAll('template');

    templates.forEach(template => {
      const name = template.getAttribute('name');
      const description = template.getAttribute('description') || '';
      const enabled = template.getAttribute('enabled') !== 'false';
      const deleted = template.getAttribute('deleted') === 'true';
      const context = template.getAttribute('context') || 'sql';

      // Skip deleted or disabled templates
      if (deleted || !enabled) {
        return;
      }

      // Only import SQL context templates
      if (context !== 'sql') {
        return;
      }

      // Get the content (text content of the template element)
      let content = template.textContent || '';

      // Skip empty templates
      if (!name || !content.trim()) {
        return;
      }

      snippets.push({
        name,
        content: content.trim(),
        description,
        context,
      });
    });
  } catch (error) {
    console.error('Failed to parse DBeaver templates XML:', error);
  }

  return snippets;
}

/**
 * Convert DBeaver snippets to our CreateSnippetInput format
 * 
 * Conversions:
 * - ${cursor} -> ${cursor} (same)
 * - $variable_name$ -> ${1:variable_name} (convert DBeaver variable placeholders)
 * - &lt; &gt; &amp; -> < > & (HTML entity decoding)
 */
export function convertDbeaverSnippets(dbeaverSnippets: DbeaverSnippet[]): CreateSnippetInput[] {
  const snippets: CreateSnippetInput[] = [];
  const seenTriggers = new Set<string>();

  dbeaverSnippets.forEach(snippet => {
    // Skip duplicates (DBeaver can have duplicate template names)
    if (seenTriggers.has(snippet.name.toLowerCase())) {
      return;
    }
    seenTriggers.add(snippet.name.toLowerCase());

    // Convert content
    let content = snippet.content;

    // Decode HTML entities
    content = decodeHtmlEntities(content);

    // Convert DBeaver variable placeholders $variable_name$ to Monaco/VS Code snippet format
    // Match pattern: $word$ but not ${cursor} or ${1:text}
    let placeholderIndex = 1;
    content = content.replace(/\$([a-zA-Z_][a-zA-Z0-9_]*)\$/g, (_, varName) => {
      return `\${${placeholderIndex++}:${varName}}`;
    });

    // Try to infer category from the trigger name or content
    const category = inferCategory(snippet.name, content);

    snippets.push({
      trigger: snippet.name,
      name: generateDisplayName(snippet.name),
      content,
      description: snippet.description || null,
      category,
    });
  });

  return snippets;
}

/**
 * Decode HTML entities commonly found in DBeaver XML
 */
function decodeHtmlEntities(text: string): string {
  return text
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#(\d+);/g, (_, num) => String.fromCharCode(parseInt(num, 10)))
    .replace(/&#x([0-9A-Fa-f]+);/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)));
}

/**
 * Generate a display name from a trigger abbreviation
 */
function generateDisplayName(trigger: string): string {
  // Common SQL abbreviations mapping
  const abbreviations: Record<string, string> = {
    'sel': 'SELECT',
    'selt': 'SELECT TOP',
    'selc': 'SELECT COUNT',
    'seld': 'SELECT DISTINCT',
    'selw': 'SELECT WHERE',
    'selj': 'SELECT JOIN',
    'ins': 'INSERT',
    'inss': 'INSERT SELECT',
    'upd': 'UPDATE',
    'del': 'DELETE',
    'trunc': 'TRUNCATE',
    'ct': 'CREATE TABLE',
    'ci': 'CREATE INDEX',
    'cni': 'CREATE NONCLUSTERED INDEX',
    'cci': 'CREATE CLUSTERED INDEX',
    'cp': 'CREATE PROCEDURE',
    'cv': 'CREATE VIEW',
    'cf': 'CREATE FUNCTION',
    'cif': 'CREATE INLINE FUNCTION',
    'ctf': 'CREATE TABLE FUNCTION',
    'csf': 'CREATE SCALAR FUNCTION',
    'cu': 'CREATE USER',
    'cl': 'CREATE LOGIN',
    'cdb': 'CREATE DATABASE',
    'ap': 'ALTER PROCEDURE',
    'af': 'ALTER FUNCTION',
    'at': 'ALTER TABLE',
    'ata': 'ALTER TABLE ADD',
    'atd': 'ALTER TABLE DROP',
    'atac': 'ALTER TABLE ALTER COLUMN',
    'atdc': 'ALTER TABLE DROP CONSTRAINT',
    'atdt': 'ALTER TABLE DISABLE TRIGGER',
    'atet': 'ALTER TABLE ENABLE TRIGGER',
    'cte': 'CTE',
    'rcte': 'Recursive CTE',
    'tran': 'Transaction',
    'tc': 'TRY CATCH',
    'decl': 'DECLARE',
    'dect': 'DECLARE TABLE',
    'exec': 'EXEC',
    'case': 'CASE',
    'be': 'BEGIN END',
    'iff': 'IF EXISTS',
    'ifn': 'IF NOT EXISTS',
    'wh': 'WHILE',
    'curff': 'CURSOR FAST_FORWARD',
  };

  if (abbreviations[trigger.toLowerCase()]) {
    return abbreviations[trigger.toLowerCase()];
  }

  // Convert camelCase or snake_case to Title Case
  return trigger
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, c => c.toUpperCase());
}

/**
 * Infer category from trigger name or content
 */
function inferCategory(trigger: string, content: string): string | null {
  const lowerTrigger = trigger.toLowerCase();
  const lowerContent = content.toLowerCase();

  // Check trigger prefix patterns
  if (lowerTrigger.startsWith('sel')) return 'Select';
  if (lowerTrigger.startsWith('ins')) return 'Insert';
  if (lowerTrigger.startsWith('upd')) return 'Update';
  if (lowerTrigger.startsWith('del') || lowerTrigger === 'trunc') return 'Delete';
  if (lowerTrigger.startsWith('ct') || lowerTrigger.startsWith('ci') || 
      lowerTrigger.startsWith('cni') || lowerTrigger.startsWith('cci') ||
      lowerTrigger.startsWith('cdb') || lowerTrigger.startsWith('di')) return 'DDL';
  if (lowerTrigger.startsWith('cp') || lowerTrigger.startsWith('ap') || 
      lowerTrigger === 'exec') return 'Procedure';
  if (lowerTrigger.startsWith('cv') || lowerTrigger.startsWith('cf') || 
      lowerTrigger.startsWith('af') || lowerTrigger.startsWith('cif') ||
      lowerTrigger.startsWith('ctf') || lowerTrigger.startsWith('csf')) return 'Function';
  if (lowerTrigger.startsWith('at')) return 'DDL';
  if (lowerTrigger.startsWith('cu') || lowerTrigger.startsWith('cl')) return 'Security';
  if (lowerTrigger === 'cte' || lowerTrigger === 'rcte') return 'CTE';
  if (lowerTrigger === 'tran' || lowerTrigger === 'tc') return 'Transaction';
  if (lowerTrigger.startsWith('decl') || lowerTrigger.startsWith('dect')) return 'Variable';
  if (lowerTrigger === 'case' || lowerTrigger === 'iff' || lowerTrigger === 'ifn' ||
      lowerTrigger === 'wh' || lowerTrigger === 'be') return 'Control Flow';
  if (lowerTrigger.startsWith('cur')) return 'Cursor';
  if (lowerTrigger.startsWith('chk') || lowerTrigger.startsWith('check') ||
      lowerTrigger.startsWith('get') || lowerTrigger.startsWith('find')) return 'Utility';

  // Check content patterns
  if (lowerContent.startsWith('select')) return 'Select';
  if (lowerContent.startsWith('insert')) return 'Insert';
  if (lowerContent.startsWith('update')) return 'Update';
  if (lowerContent.startsWith('delete')) return 'Delete';
  if (lowerContent.startsWith('create table')) return 'DDL';
  if (lowerContent.startsWith('create procedure') || lowerContent.startsWith('create proc')) return 'Procedure';
  if (lowerContent.startsWith('alter')) return 'DDL';
  if (lowerContent.startsWith('drop')) return 'DDL';
  if (lowerContent.includes('begin transaction') || lowerContent.includes('commit') || 
      lowerContent.includes('rollback')) return 'Transaction';
  if (lowerContent.startsWith('declare')) return 'Variable';
  if (lowerContent.startsWith(';with') || lowerContent.includes('with ')) return 'CTE';

  return 'Imported';
}

/**
 * Parse and convert DBeaver templates XML file to our snippet format
 */
export function importDbeaverTemplates(xmlContent: string): CreateSnippetInput[] {
  const dbeaverSnippets = parseDbeaverTemplatesXml(xmlContent);
  return convertDbeaverSnippets(dbeaverSnippets);
}
