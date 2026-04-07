import { useEffect, useMemo, useState } from 'react';

import { ThemeContext } from './theme-context';
import type { ResolvedTheme, ThemePreference } from './types';

const THEME_STORAGE_KEY = 'rustdesk-dashboard-theme';
const DARK_MEDIA_QUERY = '(prefers-color-scheme: dark)';

function isThemePreference(value: string | null): value is ThemePreference {
  return value === 'system' || value === 'light' || value === 'dark';
}

function getSystemTheme(mediaQuery?: MediaQueryList): ResolvedTheme {
  if (mediaQuery) {
    return mediaQuery.matches ? 'dark' : 'light';
  }
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return 'light';
  }
  return window.matchMedia(DARK_MEDIA_QUERY).matches ? 'dark' : 'light';
}

function resolveTheme(
  preference: ThemePreference,
  mediaQuery?: MediaQueryList,
): ResolvedTheme {
  if (preference === 'system') {
    return getSystemTheme(mediaQuery);
  }
  return preference;
}

function readStoredPreference(): ThemePreference {
  if (typeof window === 'undefined') {
    return 'system';
  }

  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    return isThemePreference(stored) ? stored : 'system';
  } catch {
    return 'system';
  }
}

function applyThemeToDocument(resolvedTheme: ResolvedTheme) {
  document.documentElement.dataset.theme = resolvedTheme;
  document.documentElement.style.colorScheme = resolvedTheme;
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreference] = useState<ThemePreference>(() => readStoredPreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveTheme(readStoredPreference()),
  );

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      setResolvedTheme(resolveTheme(preference));
      return undefined;
    }

    const mediaQuery = window.matchMedia(DARK_MEDIA_QUERY);
    const syncResolvedTheme = () => {
      setResolvedTheme(resolveTheme(preference, mediaQuery));
    };

    syncResolvedTheme();

    if (preference !== 'system') {
      return undefined;
    }

    mediaQuery.addEventListener('change', syncResolvedTheme);
    return () => mediaQuery.removeEventListener('change', syncResolvedTheme);
  }, [preference]);

  useEffect(() => {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, preference);
    } catch {
      // Ignore storage errors and keep the in-memory preference.
    }
  }, [preference]);

  useEffect(() => {
    applyThemeToDocument(resolvedTheme);
  }, [resolvedTheme]);

  const value = useMemo(
    () => ({
      preference,
      resolvedTheme,
      setPreference,
    }),
    [preference, resolvedTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
