import { useState } from 'react';
import type { FormEvent } from 'react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';

import { ApiError } from '../api';
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

  const [username, setUsername] = useState('supervisor');
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
      <form className="panel login-panel" onSubmit={onSubmit}>
        <h1>Ingreso Supervisor</h1>
        <p>Acceso al panel de monitoreo de sesiones RustDesk.</p>

        <label htmlFor="username">Usuario</label>
        <input
          id="username"
          autoComplete="username"
          value={username}
          onChange={(event) => setUsername(event.target.value)}
          required
        />

        <label htmlFor="password">Clave</label>
        <input
          id="password"
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          required
        />

        {error && <p className="error-text">{error}</p>}

        <button type="submit" className="btn primary" disabled={submitting}>
          {submitting ? 'Ingresando...' : 'Ingresar'}
        </button>
      </form>
    </div>
  );
}
