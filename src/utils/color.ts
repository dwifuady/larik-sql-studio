/**
 * Pick a readable foreground color (near-black or white) for text/icons placed
 * on top of the given background color. Uses the WCAG relative-luminance
 * formula so light accent colors get dark text and dark accents get white.
 *
 * Falls back to white when the input can't be parsed as a hex color (e.g. a
 * CSS variable like "var(--accent-color)").
 */
export function getReadableTextColor(background: string): string {
  const rgb = parseHexColor(background);
  if (!rgb) return '#ffffff';

  const [r, g, b] = rgb.map((c) => {
    const channel = c / 255;
    return channel <= 0.03928
      ? channel / 12.92
      : Math.pow((channel + 0.055) / 1.055, 2.4);
  });

  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  // Threshold ~0.5 balances contrast against typical accent hues.
  return luminance > 0.5 ? '#1a1a1a' : '#ffffff';
}

/** Parse "#rgb" / "#rrggbb" into [r, g, b]; returns null for anything else. */
function parseHexColor(value: string): [number, number, number] | null {
  const hex = value.trim().replace(/^#/, '');
  if (hex.length === 3) {
    const r = parseInt(hex[0] + hex[0], 16);
    const g = parseInt(hex[1] + hex[1], 16);
    const b = parseInt(hex[2] + hex[2], 16);
    return [r, g, b];
  }
  if (hex.length === 6) {
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    if ([r, g, b].some(Number.isNaN)) return null;
    return [r, g, b];
  }
  return null;
}
