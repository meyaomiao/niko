import { useEffect, useState } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import Login from "./pages/Login";
import Home from "./pages/Home";
import Targets from "./pages/Targets";
import Usage from "./pages/Usage";
import Settings from "./pages/Settings";
import Devices from "./pages/Devices";
import ForceUpgrade from "./pages/ForceUpgrade";
import { loadAuth, saveAuth } from "./store/auth";
import { api } from "./api/client";

const APP_VERSION = "0.1.0";

/** semver: a < b → true */
function semverLt(a: string, b: string): boolean {
  const parse = (s: string) =>
    s.replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
  const [a0, a1, a2] = parse(a);
  const [b0, b1, b2] = parse(b);
  if (a0 !== b0) return a0 < b0;
  if (a1 !== b1) return a1 < b1;
  return a2 < b2;
}

interface UpgradeInfo {
  minVersion: string;
  downloadUrl: string;
  announcement?: string;
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const auth = loadAuth();
  if (!auth?.accessToken) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

/** 登录后立即检查版本，如需强制升级则渲染升级页而非内容 */
function VersionGate({ children }: { children: React.ReactNode }) {
  const auth = loadAuth();
  const [upgradeInfo, setUpgradeInfo] = useState<UpgradeInfo | null>(null);
  const [checked, setChecked] = useState(false);

  useEffect(() => {
    if (!auth?.accessToken) { setChecked(true); return; }
    api.bootstrap(auth.accessToken)
      .then((data) => {
        // persist latest quota/group
        saveAuth({ ...auth, quota: data.user.quota, group: data.user.group });
        const minVer = data.min_supported_version ?? "0.0.0";
        if (semverLt(APP_VERSION, minVer)) {
          setUpgradeInfo({
            minVersion: minVer,
            downloadUrl: data.download_url ?? "https://momotoken.win/download",
            announcement: data.announcement?.content,
          });
        }
      })
      .catch(() => { /* 网络失败时放行，不拦截 */ })
      .finally(() => setChecked(true));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (!checked) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-950">
        <span className="text-sm text-gray-400">加载中…</span>
      </div>
    );
  }

  if (upgradeInfo) {
    return (
      <ForceUpgrade
        currentVersion={APP_VERSION}
        minVersion={upgradeInfo.minVersion}
        downloadUrl={upgradeInfo.downloadUrl}
        announcement={upgradeInfo.announcement}
      />
    );
  }

  return <>{children}</>;
}

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<Navigate to="/login" replace />} />
      <Route path="/login" element={<Login />} />
      <Route
        path="/*"
        element={
          <RequireAuth>
            <VersionGate>
              <Routes>
                <Route path="/home" element={<Home />} />
                <Route path="/targets" element={<Targets />} />
                <Route path="/usage" element={<Usage />} />
                <Route path="/settings" element={<Settings />} />
                <Route path="/devices" element={<Devices />} />
                <Route path="*" element={<Navigate to="/home" replace />} />
              </Routes>
            </VersionGate>
          </RequireAuth>
        }
      />
    </Routes>
  );
}
