import { useCallback, useEffect, useMemo, useState } from 'react';

import { ApiError, apiLogin, apiLogout, apiMe } from './api';
import { AuthContext } from './auth-context';
import type { AuthLoginResponse, AuthUser } from './types';

const AUTH_REFRESH_INTERVAL_MS = 5 * 60 * 1000;

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<AuthUser | null>(null);
  const [expiresAt, setExpiresAt] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const applyAuthState = useCallback((response: AuthLoginResponse | null) => {
    setUser(response?.user ?? null);
    setExpiresAt(response?.expires_at ?? null);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const response = await apiMe();
      applyAuthState(response);
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        applyAuthState(null);
        return;
      }
      throw error;
    }
  }, [applyAuthState]);

  useEffect(() => {
    let mounted = true;

    refresh()
      .catch((error) => {
        if (!mounted) {
          return;
        }
        console.error('auth check failed', error);
        applyAuthState(null);
      })
      .finally(() => {
        if (mounted) {
          setLoading(false);
        }
      });

    return () => {
      mounted = false;
    };
  }, [applyAuthState, refresh]);

  useEffect(() => {
    if (!user) {
      return undefined;
    }

    const timer = window.setInterval(() => {
      refresh().catch((error) => {
        if (error instanceof ApiError && error.status === 401) {
          applyAuthState(null);
          return;
        }
        console.error('background auth refresh failed', error);
      });
    }, AUTH_REFRESH_INTERVAL_MS);

    return () => window.clearInterval(timer);
  }, [applyAuthState, refresh, user]);

  const login = useCallback(
    async (username: string, password: string) => {
      const response = await apiLogin({ username, password });
      applyAuthState(response);
    },
    [applyAuthState],
  );

  const logout = useCallback(async () => {
    await apiLogout();
    applyAuthState(null);
  }, [applyAuthState]);

  const value = useMemo(
    () => ({
      user,
      expiresAt,
      loading,
      login,
      logout,
      refresh,
    }),
    [user, expiresAt, loading, login, logout, refresh],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}
