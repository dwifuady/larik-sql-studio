import { describe, test, expect } from 'vitest';
import { formatExecutionTime } from './formatters';

describe('formatExecutionTime', () => {
    test('formats milliseconds correctly', () => {
        expect(formatExecutionTime(500)).toBe('500ms');
        expect(formatExecutionTime(999)).toBe('999ms');
    });

    test('formats seconds correctly', () => {
        expect(formatExecutionTime(1000)).toBe('1s');
        expect(formatExecutionTime(1500)).toBe('1.5s');
        expect(formatExecutionTime(12345)).toBe('12.35s');
        expect(formatExecutionTime(59999)).toBe('60s'); // rounding might make it 60s or keep it as s
    });

    test('formats minutes correctly', () => {
        expect(formatExecutionTime(60000)).toBe('1m');
        expect(formatExecutionTime(90000)).toBe('1.5m');
        expect(formatExecutionTime(120000)).toBe('2m');
        expect(formatExecutionTime(3600000)).toBe('60m');
    });
});
