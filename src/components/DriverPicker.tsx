// Database driver selector shared by the space connection forms
import type { DriverKind } from '../types';
import { DRIVER_LABELS, defaultPortFor } from '../types';
import { databaseFromConnectionString, isMongoConnectionString } from '../utils/mongo/shell';

const DRIVERS = Object.keys(DRIVER_LABELS) as DriverKind[];

interface DriverPickerProps {
  value: DriverKind;
  onChange: (driver: DriverKind) => void;
}

export function DriverPicker({ value, onChange }: DriverPickerProps) {
  return (
    <div role="radiogroup" aria-label="Database type" className="grid grid-cols-2 gap-1 p-0.5 mb-2 bg-white/5 rounded">
      {DRIVERS.map(driver => (
        <button
          key={driver}
          type="button"
          role="radio"
          aria-checked={value === driver}
          onClick={() => onChange(driver)}
          className={`px-2 py-1 rounded text-xs transition-colors ${value === driver
            ? 'bg-[var(--accent-color)] text-white font-medium'
            : 'text-[var(--text-secondary)] hover:bg-white/10'
            }`}
        >
          {DRIVER_LABELS[driver]}
        </button>
      ))}
    </div>
  );
}

/**
 * Switch a connection form to another driver. The port follows the driver's
 * default unless the user typed a custom one.
 */
export function withDriver<T extends { driver: DriverKind; port: string }>(form: T, driver: DriverKind): T {
  const portIsDefault = !form.port.trim() || form.port.trim() === String(defaultPortFor(form.driver));
  return {
    ...form,
    driver,
    port: portIsDefault ? String(defaultPortFor(driver)) : form.port,
  };
}

/** Host field placeholder for a driver */
export function hostPlaceholder(driver: DriverKind): string {
  return driver === 'mongodb' ? 'Host or mongodb+srv:// string' : 'Host / Server';
}

/**
 * Update the host field. Pasting a mongodb:// or mongodb+srv:// string switches
 * the form to MongoDB and fills the database from the string's path.
 */
export function withHost<T extends { driver: DriverKind; port: string; host: string; database: string }>(form: T, host: string): T {
  let next = { ...form, host };
  if (isMongoConnectionString(host)) {
    if (next.driver !== 'mongodb') next = withDriver(next, 'mongodb');
    const database = databaseFromConnectionString(host);
    if (database && !form.database.trim()) next = { ...next, database };
  }
  return next;
}

/** Readable message from a failed connection test */
export function connectionErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes('Password expired')) return 'Password expired. Please change it using another tool.';
  return `Failed to connect: ${message}`;
}

/**
 * Whether the form holds enough to save and test a connection.
 * MongoDB only needs a host: databases are listed once connected.
 */
export function connectionIsComplete(form: { driver: DriverKind; host: string; database: string }): boolean {
  if (!form.host.trim()) return false;
  return form.driver === 'mongodb' || !!form.database.trim();
}

/** Database field placeholder for a driver */
export function databasePlaceholder(driver: DriverKind): string {
  return driver === 'mongodb' ? 'Default database (optional)' : 'Database';
}

/** Explains why the connection isn't saved or testable yet */
export function MissingDatabaseHint({ connection }: { connection: { driver: DriverKind; host: string; database: string } }) {
  if (!connection.host.trim() || connectionIsComplete(connection)) return null;
  return (
    <p className="text-xs text-[var(--text-secondary)] mb-2">
      Enter a database to save and test this connection.
    </p>
  );
}
