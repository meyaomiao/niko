import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { open } from "@tauri-apps/plugin-shell";
import { invoke } from "@tauri-apps/api/core";
import { api, REGISTER_URL, type SiteConfig } from "../api/client";
import { saveAuth } from "../store/auth";

// 设备信息
function getDeviceId(): string {
  let id = localStorage.getItem("momo_device_id");
  if (!id) {
    id = crypto.randomUUID();
    localStorage.setItem("momo_device_id", id);
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

type Stage = "login" | "2fa";

export default function Login() {
  const navigate = useNavigate();
  const [site, setSite] = useState<SiteConfig | null>(null);
  const [stage, setStage] = useState<Stage>("login");
  const [pendingToken, setPendingToken] = useState("");

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");

  const [remember, setRemember] = useState(false);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  // 加载站点配置
  useEffect(() => {
    api.getSite().then(setSite).catch(() => setSite({ system_name: "momo·摸摸", server_version: "" }));
  }, []);

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
      setError(err instanceof Error ? err.message : "验证失败");
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
          <div className="mb-2 text-4xl">🐾</div>
          <h1 className="text-2xl font-bold text-gray-900 dark:text-white">
            {site?.system_name ?? "momo·摸摸"}
          </h1>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400 dark:text-gray-400">
            {stage === "login" ? "登录账号以使用 API 服务" : "请输入两步验证码"}
          </p>
        </div>

        {error && (
          <div className="mb-4 rounded-lg bg-red-500/10 px-4 py-2 text-sm text-red-600 dark:text-red-300">
            {error}
          </div>
        )}

        {stage === "login" ? (
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
