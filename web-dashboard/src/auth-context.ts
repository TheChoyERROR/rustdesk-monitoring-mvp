import { createContext } from 'react';

import type { AuthUser } from './types';

export interface AuthContextValue {
  user: AuthUser | null;
  expiresAt: string | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
}

export const AuthContext = createContext<AuthContextValue | undefined>(undefined);
