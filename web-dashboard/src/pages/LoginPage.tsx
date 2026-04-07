import { useState } from 'react';
import type { FormEvent } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';

import { ApiError } from '../api';
import ThemeToggle from '../components/ThemeToggle';
import { useAuth } from '../useAuth';

interface LocationState {
  from?: string;
}

export default function LoginPage() {
  const { user, login } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const state = location.state as LocationState | null;
  const redirectTo = state?.from ?? '/';

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (user) {
    return <Navigate to={redirectTo} replace />;
  }

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSubmitting(true);
    setError(null);

    try {
      await login(username, password);
      navigate(redirectTo, { replace: true });
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        setError('Usuario o clave invalidos.');
      } else {
        setError('No se pudo iniciar sesion. Revisa el backend.');
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="centered-screen">
      <div className="login-screen">
        <div className="login-toolbar">
          <ThemeToggle />
        </div>
        <div className="login-layout">
          <section className="login-hero">
            <span className="login-badge">RustDesk Monitoring</span>
            <h1>Centro de supervision y helpdesk operativo</h1>
            <p>
              Accede al tablero para coordinar tickets, agentes autorizados y actividad remota
              desde una sola vista.
            </p>

            <div className="login-hero-grid">
              <article className="login-hero-card">
                <strong>Helpdesk</strong>
                <span>Despacho manual, auditoria y control supervisor.</span>
              </article>
              <article className="login-hero-card">
                <strong>Monitoreo</strong>
                <span>Timeline de sesiones, presencia y eventos en vivo.</span>
              </article>
              <article className="login-hero-card">
                <strong>Operacion</strong>
                <span>Tickets, agentes y persistencia inicial con Turso.</span>
              </article>
            </div>
          </section>

          <form className="panel login-panel login-card" onSubmit={onSubmit}>
            <div className="login-card-header">
              <h2>Ingreso supervisor</h2>
              <p>Usa tu usuario y clave del dashboard.</p>
            </div>

            <div className="field-group">
              <label htmlFor="username">Usuario</label>
              <input
                id="username"
                autoComplete="username"
                value={username}
                placeholder="supervisor"
                onChange={(event) => setUsername(event.target.value)}
                required
                autoFocus
              />
            </div>

            <div className="field-group">
              <label htmlFor="password">Clave</label>
              <input
                id="password"
                type="password"
                autoComplete="current-password"
                value={password}
                placeholder="Ingresa tu clave"
                onChange={(event) => setPassword(event.target.value)}
                required
              />
            </div>

            {error && <p className="error-text login-error">{error}</p>}

            <button type="submit" className="btn primary login-submit" disabled={submitting}>
              {submitting ? 'Ingresando...' : 'Ingresar al panel'}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
