import { Navigate, Route, Routes } from 'react-router-dom';

import AppShell from './components/AppShell';
import ProtectedRoute from './components/ProtectedRoute';
import HelpdeskPage from './pages/HelpdeskPage';
import HelpdeskTicketDetailPage from './pages/HelpdeskTicketDetailPage';
import LoginPage from './pages/LoginPage';
import SessionDetailPage from './pages/SessionDetailPage';
import SessionsPage from './pages/SessionsPage';
import SummaryPage from './pages/SummaryPage';

export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<ProtectedRoute />}>
        <Route element={<AppShell />}>
          <Route path="/" element={<SummaryPage />} />
          <Route path="/helpdesk" element={<HelpdeskPage />} />
          <Route path="/helpdesk/tickets/:ticketId" element={<HelpdeskTicketDetailPage />} />
          <Route path="/sessions" element={<SessionsPage />} />
          <Route path="/sessions/:sessionId" element={<SessionDetailPage />} />
        </Route>
      </Route>
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
