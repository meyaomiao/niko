import { Routes, Route, Navigate } from "react-router-dom";
import Login from "./pages/Login";
import Home from "./pages/Home";
import Targets from "./pages/Targets";
import Usage from "./pages/Usage";
import Settings from "./pages/Settings";

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<Navigate to="/login" replace />} />
      <Route path="/login" element={<Login />} />
      <Route path="/home" element={<Home />} />
      <Route path="/targets" element={<Targets />} />
      <Route path="/usage" element={<Usage />} />
      <Route path="/settings" element={<Settings />} />
    </Routes>
  );
}
