import { useEffect, useState } from 'react';
import { NavLink, Outlet, useNavigate } from 'react-router-dom';

import {
  getHelpdeskBackendMode,
  setHelpdeskBackendMode,
  type HelpdeskBackendMode,
} from '../api';
import { useAuth } from '../useAuth';
import ThemeToggle from './ThemeToggle';

export default function AppShell() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const [helpdeskBackendMode, setHelpdeskBackendModeState] =
    useState<HelpdeskBackendMode>(getHelpdeskBackendMode());

  useEffect(() => {
    const syncMode = () => {
      setHelpdeskBackendModeState(getHelpdeskBackendMode());
    };

    const onStorage = (event: StorageEvent) => {
      if (event.key === 'helpdesk_backend_mode') {
        syncMode();
      }
    };

    window.addEventListener('helpdesk-backend-mode-changed', syncMode);
    window.addEventListener('storage', onStorage);

    return () => {
      window.removeEventListener('helpdesk-backend-mode-changed', syncMode);
      window.removeEventListener('storage', onStorage);
    };
  }, []);

  const onLogout = async () => {
    await logout();
    navigate('/login', { replace: true });
  };

  const toggleHelpdeskBackendMode = () => {
    const nextMode: HelpdeskBackendMode =
      helpdeskBackendMode === 'sqlite' ? 'postgres' : 'sqlite';
    setHelpdeskBackendMode(nextMode);
    setHelpdeskBackendModeState(nextMode);
  };

  return (
    <div className="app-shell">
      <header className="app-header">
        <div>
          <h1>RustDesk Monitoring</h1>
          <p>Panel de supervision operativa</p>
        </div>
        <div className="header-actions">
          <ThemeToggle />
          <button
            type="button"
            className="btn secondary"
            onClick={toggleHelpdeskBackendMode}
            title="Alterna entre el flujo actual en SQLite y el flujo experimental de helpdesk en Postgres."
          >
            Helpdesk: {helpdeskBackendMode === 'postgres' ? 'Postgres' : 'SQLite'}
          </button>
          <span className="chip">{user?.username}</span>
          <button type="button" onClick={onLogout} className="btn secondary">
            Cerrar sesion
          </button>
        </div>
      </header>

      <nav className="app-nav">
        <NavLink to="/" end className={({ isActive }) => (isActive ? 'nav-link active' : 'nav-link')}>
          Resumen
        </NavLink>
        <NavLink
          to="/helpdesk"
          className={({ isActive }) => (isActive ? 'nav-link active' : 'nav-link')}
        >
          Helpdesk
        </NavLink>
        <NavLink
          to="/sessions"
          className={({ isActive }) => (isActive ? 'nav-link active' : 'nav-link')}
        >
          Sesiones
        </NavLink>
      </nav>

      <main className="app-main">
        <Outlet />
      </main>
    </div>
  );
}
