import { NavLink, Outlet, useNavigate } from 'react-router-dom';

import { useAuth } from '../useAuth';

export default function AppShell() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();

  const onLogout = async () => {
    await logout();
    navigate('/login', { replace: true });
  };

  return (
    <div className="app-shell">
      <header className="app-header">
        <div>
          <h1>RustDesk Monitoring</h1>
          <p>Panel de supervision operativa</p>
        </div>
        <div className="header-actions">
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
