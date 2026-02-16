/**
 * Cell content formatter utilities
 * Detects and formats JSON, XML, or plain text content
 */

export type CellFormat = 'json' | 'xml' | 'plain' | 'null' | 'binary';

export type CellValue = string | number | boolean | null | Uint8Array;

/**
 * Detects the format of a cell value
 * Priority: NULL > Binary > JSON > XML > Plain
 */
export function detectCellFormat(value: CellValue): CellFormat {
  // Handle NULL
  if (value === null || value === undefined) {
    return 'null';
  }

  // Handle binary data
  if (value instanceof Uint8Array || (Array.isArray(value) && value.length > 0 && typeof value[0] === 'number')) {
    return 'binary';
  }

  // Convert to string for text-based detection
  const str = String(value).trim();

  // Empty string
  if (str === '') {
    return 'plain';
  }

  // Detect JSON
  if ((str.startsWith('{') || str.startsWith('['))) {
    try {
      JSON.parse(str);
      return 'json';
    } catch {
      // Not valid JSON, continue checking
    }
  }

  // Detect XML
  if (str.startsWith('<')) {
    // Basic XML validation: has opening and closing tags
    const tagMatch = str.match(/<(\w+)[^>]*>/);
    if (tagMatch) {
      const tagName = tagMatch[1];
      const hasClosingTag = str.includes(`</${tagName}>`);
      if (hasClosingTag) {
        return 'xml';
      }
    }
  }

  return 'plain';
}

/**
 * Formats cell content based on the specified format type
 * Returns formatted string ready for display in Monaco editor
 */
export function formatCellContent(
  value: CellValue,
  format: 'auto' | 'json' | 'xml' | 'plain'
): { content: string; language: 'json' | 'xml' | 'plaintext'; error?: string } {
  // Handle NULL
  if (value === null || value === undefined) {
    return {
      content: '',
      language: 'plaintext',
      error: 'NULL value'
    };
  }

  // Handle binary data
  if (value instanceof Uint8Array || (Array.isArray(value) && value.length > 0 && typeof value[0] === 'number')) {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
    return {
      content: formatBinaryData(bytes),
      language: 'plaintext',
      error: `Binary data: ${bytes.length} bytes`
    };
  }

  const str = String(value);

  // Auto-detect if format is 'auto'
  const actualFormat = format === 'auto' ? detectCellFormat(value) : format;

  // Format based on type
  switch (actualFormat) {
    case 'json':
      return formatJSON(str);
    case 'xml':
      return formatXML(str);
    case 'plain':
    default:
      return {
        content: str,
        language: 'plaintext'
      };
  }
}

/**
 * Formats JSON with proper indentation
 */
function formatJSON(str: string): { content: string; language: 'json'; error?: string } {
  try {
    const parsed = JSON.parse(str);
    const formatted = JSON.stringify(parsed, null, 2);
    return {
      content: formatted,
      language: 'json'
    };
  } catch (error) {
    return {
      content: str,
      language: 'json',
      error: `Invalid JSON: ${error instanceof Error ? error.message : 'Parse error'}`
    };
  }
}

/**
 * Formats XML with basic indentation
 */
function formatXML(str: string): { content: string; language: 'xml'; error?: string } {
  try {
    // Basic XML formatting using regex
    let formatted = str.replace(/>\s*</g, '>\n<');

    // Add indentation
    const lines = formatted.split('\n');
    let indent = 0;
    const indented: string[] = [];

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;

      // Handle closing tags - decrease indent before adding line
      if (trimmed.startsWith('</')) {
        indent = Math.max(0, indent - 1);
        indented.push('  '.repeat(indent) + trimmed);
        continue;
      }

      // Handle self-closing tags - no indent change
      if (trimmed.endsWith('/>')) {
        indented.push('  '.repeat(indent) + trimmed);
        continue;
      }

      // Handle opening tags with closing tag on same line (e.g., <tag>content</tag>)
      const openTagMatch = trimmed.match(/^<([^/?!\s>]+)[^>]*>/);
      if (openTagMatch) {
        const tagName = openTagMatch[1];
        const closingTagPattern = new RegExp(`</${tagName}>\\s*$`);

        if (closingTagPattern.test(trimmed)) {
          // Opening and closing on same line - no indent change
          indented.push('  '.repeat(indent) + trimmed);
          continue;
        }

        // Opening tag only - add line then increase indent
        indented.push('  '.repeat(indent) + trimmed);
        indent++;
        continue;
      }

      // Default case - just add the line
      indented.push('  '.repeat(indent) + trimmed);
    }

    return {
      content: indented.join('\n'),
      language: 'xml'
    };
  } catch (error) {
    return {
      content: str,
      language: 'xml',
      error: `XML formatting error: ${error instanceof Error ? error.message : 'Unknown error'}`
    };
  }
}

/**
 * Formats binary data as hex dump (first 1KB)
 */
function formatBinaryData(bytes: Uint8Array): string {
  const maxBytes = Math.min(bytes.length, 1024); // Show first 1KB
  const lines: string[] = [];

  for (let i = 0; i < maxBytes; i += 16) {
    const offset = i.toString(16).padStart(8, '0');
    const chunk = bytes.slice(i, i + 16);

    // Hex representation
    const hex = Array.from(chunk)
      .map(b => b.toString(16).padStart(2, '0'))
      .join(' ')
      .padEnd(47, ' '); // 16 bytes * 2 chars + 15 spaces = 47

    // ASCII representation
    const ascii = Array.from(chunk)
      .map(b => (b >= 32 && b <= 126 ? String.fromCharCode(b) : '.'))
      .join('');

    lines.push(`${offset}  ${hex}  ${ascii}`);
  }

  if (bytes.length > maxBytes) {
    lines.push('');
    lines.push(`... ${bytes.length - maxBytes} more bytes (truncated)`);
  }

  return lines.join('\n');
}
