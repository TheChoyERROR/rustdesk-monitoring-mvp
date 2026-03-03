import { Navigate, Outlet, useLocation } from 'react-router-dom';

import { useAuth } from '../useAuth';

export default function ProtectedRoute() {
  const { user, loading } = useAuth();
  const location = useLocation();

  if (loading) {
    return (
      <div className="centered-screen">
        <div className="panel">
          <p>Validando sesion...</p>
        </div>
      </div>
    );
  }

  if (!user) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }

  return <Outlet />;
}
