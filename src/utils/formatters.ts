/**
 * Utility functions for formatting various data types for display
 */

/**
 * Formats execution time from milliseconds to a human-readable string.
 * - < 1 second: returns "Xms"
 * - < 1 minute: returns "X.XXs"
 * - >= 1 minute: returns "X.XXm"
 */
export function formatExecutionTime(ms: number): string {
    if (isNaN(ms) || ms === null || ms === undefined) {
        return '0ms';
    }

    if (ms < 1000) {
        return `${ms}ms`;
    }

    if (ms < 60000) {
        const seconds = ms / 1000;
        // Use 2 decimal places, but truncate trailing zeros
        return `${parseFloat(seconds.toFixed(2))}s`;
    }

    const minutes = ms / 60000;
    // Use 2 decimal places, but truncate trailing zeros
    return `${parseFloat(minutes.toFixed(2))}m`;
}
