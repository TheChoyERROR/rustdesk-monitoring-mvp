import { useTheme } from '../useTheme';
import type { ThemePreference } from '../types';

const THEME_OPTIONS: Array<{ value: ThemePreference; label: string }> = [
  { value: 'system', label: 'Sistema' },
  { value: 'light', label: 'Claro' },
  { value: 'dark', label: 'Oscuro' },
];

interface ThemeToggleProps {
  label?: string;
}

export default function ThemeToggle({
  label = 'Tema',
}: ThemeToggleProps) {
  const { preference, setPreference } = useTheme();

  return (
    <div className="theme-toggle">
      <span className="theme-toggle-label">{label}</span>
      <div className="theme-toggle-buttons" role="group" aria-label="Selector de tema">
        {THEME_OPTIONS.map((option) => (
          <button
            key={option.value}
            type="button"
            className="theme-toggle-button"
            aria-pressed={preference === option.value}
            onClick={() => setPreference(option.value)}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}
