import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { open } from "@tauri-apps/plugin-shell";
import { invoke } from "@tauri-apps/api/core";
import { api, DeviceLimitError, REGISTER_URL, type DeviceItem } from "../api/client";
import { saveAuth } from "../store/auth";
import { BRAND } from "../lib/brand";
import Logo from "../components/Logo";

// 设备信息
function getDeviceId(): string {
  let id = localStorage.getItem("niko_device_id");
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem("niko_device_id", id);
  }
  return id;
}
function getDeviceName(): string {
  const ua = navigator.userAgent;
  if (ua.includes("Mac")) return "macOS";
  if (ua.includes("Win")) return "Windows";
  if (ua.includes("Linux")) return "Linux";
  return "Unknown";
}

type Stage = "login" | "2fa" | "device-limit";

function formatDeviceTime(ts: number): string {
  if (!ts) return "未知";
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function Login() {
  const navigate = useNavigate();
  const [stage, setStage] = useState<Stage>("login");
  const [pendingToken, setPendingToken] = useState("");

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");

  const [remember, setRemember] = useState(false);

  // 设备数达上限：登录页当场列出旧设备供用户勾选退出，避免被挡在门外
  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [deviceLimit, setDeviceLimit] = useState(0);
  const [selectedDevices, setSelectedDevices] = useState<number[]>([]);
  // 记住触发上限时处于哪一步，释放设备后按原路重试
  const [limitFrom, setLimitFrom] = useState<Stage>("login");

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  // 回填「记住我」保存的凭证（存在系统钥匙串，不落明文文件）
  useEffect(() => {
    invoke<{ username: string; password: string } | null>("load_remembered_login")
      .then((saved) => {
        if (!saved) return;
        setUsername(saved.username);
        setPassword(saved.password);
        setRemember(true);
      })
      .catch(() => {});
  }, []);

  const handleLogin = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password.trim()) { setError("请填写账号和密码"); return; }
    setError(""); setLoading(true);
    try {
      const result = await api.login({
        username: username.trim(),
        password,
        deviceId: getDeviceId(),
        deviceName: getDeviceName(),
        platform: getDeviceName(),
      });
      if (result.require_2fa && result.pending_token) {
        setPendingToken(result.pending_token);
        setStage("2fa");
        return;
      }
      await finishLogin(result.access_token!, result.username ?? username);
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, "login");
        return;
      }
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setLoading(false);
    }
  };

  const handle2FA = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!code.trim()) { setError("请输入验证码"); return; }
    setError(""); setLoading(true);
    try {
      const result = await api.login2fa(pendingToken, code.trim());
      await finishLogin(result.access_token!, result.username ?? username);
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, "2fa");
        return;
      }
      setError(err instanceof Error ? err.message : "验证失败");
    } finally {
      setLoading(false);
    }
  };

  const enterDeviceLimit = (err: DeviceLimitError, from: Stage) => {
    setDevices(err.devices);
    setDeviceLimit(err.deviceLimit);
    setSelectedDevices([]);
    setLimitFrom(from);
    setError(err.message);
    setStage("device-limit");
  };

  // 释放勾选的旧设备后按原路重试：撤销与登录在同一请求内完成，
  // 不需要先拿到 token 才能调设备管理接口。
  const handleRevokeAndLogin = async () => {
    if (selectedDevices.length === 0) { setError("请至少选择一台要退出的设备"); return; }
    setError(""); setLoading(true);
    try {
      const result =
        limitFrom === "2fa"
          ? await api.login2fa(pendingToken, code.trim(), selectedDevices)
          : await api.login({
              username: username.trim(),
              password,
              deviceId: getDeviceId(),
              deviceName: getDeviceName(),
              platform: getDeviceName(),
              revokeSessionIds: selectedDevices,
            });
      if (result.require_2fa && result.pending_token) {
        setPendingToken(result.pending_token);
        setStage("2fa");
        return;
      }
      await finishLogin(result.access_token!, result.username ?? username);
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, limitFrom);
        return;
      }
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setLoading(false);
    }
  };

  const finishLogin = async (token: string, uname: string) => {
    // 记住我：凭证写入系统钥匙串；未勾选则清掉历史记录
    try {
      if (remember) {
        await invoke("save_remembered_login", {
          login: { username: username.trim(), password },
        });
      } else {
        await invoke("clear_remembered_login");
      }
    } catch {
      // 钥匙串不可用时不阻断登录
    }
    try {
      const bootstrap = await api.bootstrap(token);
      const provision = await api.provision(token, bootstrap.user.group);
      saveAuth({
        accessToken: token,
        username: uname,
        userId: bootstrap.user.id,
        quota: bootstrap.user.quota,
        group: bootstrap.user.group,
        apiKey: provision.api_key,
      });
      navigate("/home");
    } catch {
      // bootstrap/provision 失败不阻断登录，仍跳首页
      saveAuth({ accessToken: token, username: uname, userId: 0, quota: 0, group: "", apiKey: "" });
      navigate("/home");
    }
  };

  return (
    <div className="flex h-screen items-center justify-center bg-transparent">
      <div className="w-full max-w-sm rounded-2xl bg-white dark:bg-white/5 p-8 shadow-xl">
        <div className="mb-8 text-center">
          <div className="mb-3 flex justify-center">
            <Logo size={56} />
          </div>
          <h1 className="text-2xl font-bold text-gray-900 dark:text-white">{BRAND.name}</h1>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            {stage === "login"
              ? BRAND.tagline
              : stage === "2fa"
                ? "请输入两步验证码"
                : "选择要退出登录的设备"}
          </p>
        </div>

        {error && (
          <div className="mb-4 rounded-lg bg-red-500/10 px-4 py-2 text-sm text-red-600 dark:text-red-300">
            {error}
          </div>
        )}

        {stage === "device-limit" ? (
          <div className="space-y-4">
            <p className="text-xs text-gray-500 dark:text-gray-400">
              已登录 {devices.length}
              {deviceLimit > 0 && ` / ${deviceLimit}`} 台。勾选不再使用的设备，退出后即可继续登录。
            </p>
            <div className="max-h-56 space-y-2 overflow-y-auto">
              {devices.map((d) => {
                const checked = selectedDevices.includes(d.id);
                return (
                  <label
                    key={d.id}
                    className="flex cursor-pointer items-center gap-3 rounded-xl bg-black/[0.03] px-3 py-2 dark:bg-white/5"
                  >
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={(e) =>
                        setSelectedDevices((prev) =>
                          e.target.checked ? [...prev, d.id] : prev.filter((x) => x !== d.id)
                        )
                      }
                      disabled={loading}
                      className="h-3.5 w-3.5 rounded border-black/20 accent-indigo-600 dark:border-white/25"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                        {d.device_name || d.device_id.slice(0, 8)}
                      </span>
                      <span className="block text-xs text-gray-500 dark:text-gray-400">
                        {d.platform} · 最后活跃 {formatDeviceTime(d.accessed_time)}
                      </span>
                    </span>
                  </label>
                );
              })}
            </div>
            <button
              type="button"
              onClick={handleRevokeAndLogin}
              disabled={loading || selectedDevices.length === 0}
              className="w-full rounded-lg bg-indigo-600 py-2 text-sm font-medium text-white transition hover:bg-indigo-500 disabled:opacity-50"
            >
              {loading
                ? "处理中…"
                : `退出所选 ${selectedDevices.length} 台并登录`}
            </button>
            <button
              type="button"
              onClick={() => { setStage(limitFrom); setError(""); }}
              disabled={loading}
              className="w-full text-sm text-gray-500 transition hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200"
            >
              ← 返回
            </button>
          </div>
        ) : stage === "login" ? (
          <form onSubmit={handleLogin} className="space-y-4">
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400 dark:text-gray-400">账号</label>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="w-full rounded-lg bg-black/[0.04] dark:bg-white/10 px-3 py-2 text-sm text-gray-900 dark:text-white placeholder-gray-500 outline-none focus:ring-2 focus:ring-indigo-500"
                placeholder="用户名或邮箱"
                autoComplete="username"
                disabled={loading}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400 dark:text-gray-400">密码</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full rounded-lg bg-black/[0.04] dark:bg-white/10 px-3 py-2 text-sm text-gray-900 dark:text-white placeholder-gray-500 outline-none focus:ring-2 focus:ring-indigo-500"
                placeholder="••••••••"
                autoComplete="current-password"
                disabled={loading}
              />
            </div>

            <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-300">
              <input
                type="checkbox"
                checked={remember}
                onChange={(e) => setRemember(e.target.checked)}
                disabled={loading}
                className="h-3.5 w-3.5 rounded border-black/20 accent-indigo-600 dark:border-white/25"
              />
              记住我
            </label>

            <button
              type="submit"
              disabled={loading}
              className="w-full rounded-lg bg-indigo-600 py-2 text-sm font-medium text-gray-900 dark:text-white transition hover:bg-indigo-500 disabled:opacity-50"
            >
              {loading ? "登录中…" : "登录"}
            </button>

            {/* 登录器不提供注册，引导到官网注册页 */}
            <button
              type="button"
              onClick={() => open(REGISTER_URL)}
              className="w-full text-center text-xs text-gray-500 dark:text-gray-400 dark:text-gray-400 transition hover:text-gray-800 dark:text-gray-200"
            >
              还没有账号？前往官网注册
            </button>
          </form>
        ) : (
          <form onSubmit={handle2FA} className="space-y-4">
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400 dark:text-gray-400">6 位验证码</label>
              <input
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                maxLength={6}
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className="w-full rounded-lg bg-black/[0.04] dark:bg-white/10 px-3 py-2 text-center text-lg tracking-widest text-gray-900 dark:text-white placeholder-gray-500 outline-none focus:ring-2 focus:ring-indigo-500"
                placeholder="000000"
                autoFocus
                disabled={loading}
              />
            </div>
            <button
              type="submit"
              disabled={loading}
              className="w-full rounded-lg bg-indigo-600 py-2 text-sm font-medium text-gray-900 dark:text-white transition hover:bg-indigo-500 disabled:opacity-50"
            >
              {loading ? "验证中…" : "确认"}
            </button>
            <button
              type="button"
              onClick={() => { setStage("login"); setCode(""); setError(""); }}
              className="w-full text-sm text-gray-500 dark:text-gray-400 dark:text-gray-400 hover:text-gray-800 dark:text-gray-200"
            >
              ← 返回登录
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
