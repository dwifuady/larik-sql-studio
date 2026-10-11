// Database driver selector shared by the space connection forms
import type { DriverKind } from '../types';
import { DRIVER_LABELS, defaultPortFor } from '../types';

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
