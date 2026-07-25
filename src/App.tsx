import { Routes, Route, Navigate } from "react-router-dom";
import Login from "./pages/Login";
import Home from "./pages/Home";
import Targets from "./pages/Targets";
import Usage from "./pages/Usage";
import Settings from "./pages/Settings";
import { loadAuth } from "./store/auth";

function RequireAuth({ children }: { children: React.ReactNode }) {
  const auth = loadAuth();
  if (!auth?.accessToken) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<Navigate to="/login" replace />} />
      <Route path="/login" element={<Login />} />
      <Route path="/home" element={<RequireAuth><Home /></RequireAuth>} />
      <Route path="/targets" element={<RequireAuth><Targets /></RequireAuth>} />
      <Route path="/usage" element={<RequireAuth><Usage /></RequireAuth>} />
      <Route path="/settings" element={<RequireAuth><Settings /></RequireAuth>} />
    </Routes>
  );
}
