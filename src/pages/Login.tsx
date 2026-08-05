import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { open } from "@tauri-apps/plugin-shell";
import { invoke } from "@tauri-apps/api/core";
import { api, DeviceLimitError, type DeviceItem } from "../api/client";
import { parseBalanceSnapshot } from "../lib/balance";
import { saveAuth, shouldPersistAuthSession } from "../store/auth";
import { BRAND } from "../lib/brand";
import {
  getTargetRenderState,
  mapLoginTargets,
  type DetectionStatus,
  type LoginTarget,
  type TargetInfo,
} from "../lib/loginTargets";
import {
  createRegistrationSubmissionGate,
  registerThenLogin,
  registrationErrorMessage,
  toggleAuthMode,
  validateRegistration,
  type AuthMode,
} from "../lib/registration";
import Logo from "../components/Logo";
import TargetAppIcon from "../components/TargetAppIcon";
import { BookOpenIcon } from "../components/Icons";
import { displayDeviceLabel, friendlyLoginError } from "../lib/copy";

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

type Stage = "login" | "register" | "2fa" | "device-limit";
type LoginStage = "login" | "2fa";
type CredentialSource = "login" | "registration";
type VerificationState = "idle" | "opening" | "pending" | "verified";

interface ChallengeStart {
  nonce: string;
  expires_in_seconds: number;
}

interface ChallengeStatus {
  status: "pending" | "verified" | "expired" | "missing";
}

function formatDeviceTime(ts: number): string {
  if (!ts) return "未知";
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function RefreshIcon({ spinning = false }: { spinning?: boolean }) {
  return (
    <svg
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
      className={`h-3.5 w-3.5 ${spinning ? "animate-spin motion-reduce:animate-none" : ""}`}
    >
      <path
        d="M16.25 6.75A7 7 0 1 0 17 10"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
      <path
        d="M16.25 3.75v3h-3"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ExternalLinkIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" aria-hidden="true" className="h-3.5 w-3.5">
      <path
        d="M6 4h-2.5A1.5 1.5 0 0 0 2 5.5v7A1.5 1.5 0 0 0 3.5 14h7a1.5 1.5 0 0 0 1.5-1.5V10M8.5 2H14v5.5M13.5 2.5 7 9"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

type InstallGuideIconName = "download" | "install" | "launch" | "detect";
type InstallPlatform = "macOS" | "Windows";

interface InstallGuideStep {
  title: string;
  description: string;
  icon: InstallGuideIconName;
  tone: string;
}

function InstallGuideIcon({ name }: { name: InstallGuideIconName }) {
  const paths: Record<InstallGuideIconName, React.ReactNode> = {
    download: (
      <>
        <path d="M12 3v11" />
        <path d="m8 10 4 4 4-4" />
        <path d="M5 19h14" />
      </>
    ),
    install: (
      <>
        <rect x="3" y="4" width="18" height="16" rx="2.5" />
        <path d="M3 8h18" />
        <path d="m9 14 2 2 4-4" />
      </>
    ),
    launch: (
      <>
        <rect x="3" y="3" width="18" height="18" rx="4" />
        <path d="m10 8 6 4-6 4Z" />
      </>
    ),
    detect: (
      <>
        <path d="M20 11a8 8 0 1 0-2.3 5.7" />
        <path d="M20 5v6h-6" />
      </>
    ),
  };

  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className="h-5 w-5"
    >
      {paths[name]}
    </svg>
  );
}

function detectInstallPlatform(): InstallPlatform {
  return navigator.userAgent.includes("Windows") ? "Windows" : "macOS";
}

function getTargetShortName(target: LoginTarget): string {
  return target.id === "codex" ? "ChatGPT" : target.id === "claude-desktop" ? "Claude" : target.name;
}

function getInstallGuideSteps(target: LoginTarget, platform: InstallPlatform): InstallGuideStep[] {
  const shortName = getTargetShortName(target);
  const installDescription =
    platform === "macOS"
      ? `打开下载的安装包，将 ${shortName} 放入“应用程序”文件夹`
      : `运行下载的安装程序，按照系统提示完成 ${shortName} 安装`;

  return [
    {
      title: "打开官方下载页",
      description: `点击下方按钮，进入 ${shortName} 官方下载页面。`,
      icon: "download",
      tone: "bg-[var(--nk-info-soft)] text-[var(--nk-info)]",
    },
    {
      title: `下载 ${platform} 版本`,
      description: `在官网选择适合当前电脑的版本，等待安装包下载完成。`,
      icon: "install",
      tone: "bg-[var(--nk-warning-soft)] text-[var(--nk-warning)]",
    },
    {
      title: "完成安装并打开",
      description: `${installDescription}，安装后启动一次应用。`,
      icon: "launch",
      tone: "bg-[var(--nk-danger-soft)] text-[var(--nk-accent)]",
    },
    {
      title: "回到 Niko 重新检查",
      description: "应用打开后回到这里点击“重新检查”，显示“已安装”即可继续。",
      icon: "detect",
      tone: "bg-[var(--nk-success-soft)] text-[var(--nk-success)]",
    },
  ];
}

interface TargetPreparationProps {
  targets: LoginTarget[];
  detectionStatus: DetectionStatus;
  detectionError: string;
  actionError: string;
  onDetect: () => void;
  onOpenInstaller: (target: LoginTarget) => void;
}

function TargetPreparation({
  targets,
  detectionStatus,
  detectionError,
  actionError,
  onDetect,
  onOpenInstaller,
}: TargetPreparationProps) {
  const [selectedTargetId, setSelectedTargetId] = useState("codex");
  const installedCount = targets.filter((target) => target.installed).length;
  const missingTargets = targets.filter((target) => !target.installed);
  const selectedTarget =
    missingTargets.find((target) => target.id === selectedTargetId) ??
    missingTargets[0] ??
    targets[0];
  const platform = detectInstallPlatform();
  const guideSteps = selectedTarget ? getInstallGuideSteps(selectedTarget, platform) : [];
  const allInstalled = detectionStatus === "success" && installedCount === targets.length;
  const showGuide = detectionStatus === "success" && !allInstalled && selectedTarget;
  const statusLabel = detectionStatus === "checking"
    ? "正在检查本机应用"
    : detectionStatus === "error"
      ? "检查失败"
      : allInstalled
        ? "应用已准备好"
        : `检查完成 · 还需安装 ${missingTargets.length} 个应用`;
  const statusDot =
    detectionStatus === "checking"
      ? "text-indigo-600 dark:text-indigo-400"
      : detectionStatus === "success"
        ? "text-emerald-600 dark:text-emerald-400"
        : "text-red-600 dark:text-red-400";

  return (
    <section
      aria-labelledby="target-preparation-title"
      className="min-w-0 p-5 sm:p-7 md:flex md:min-h-0 md:flex-col md:overflow-y-auto md:p-8 lg:p-10"
    >
      <div className="mx-auto my-auto w-full max-w-5xl">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h2
            id="target-preparation-title"
            className="text-lg font-semibold text-gray-900 dark:text-gray-100"
          >
            先把要接入的应用装好
          </h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-gray-500 dark:text-gray-400">
            Niko 会自动检查 ChatGPT 和 Claude。未安装时，跟着下面四步操作即可。
          </p>
        </div>
        <button
          type="button"
          onClick={onDetect}
          disabled={detectionStatus === "checking"}
          className="nk-btn-secondary"
        >
          <RefreshIcon spinning={detectionStatus === "checking"} />
          {detectionStatus === "checking" ? "检查中" : "重新检查"}
        </button>
      </div>

      <div
        role="status"
        aria-live="polite"
        className={`mt-4 flex items-center gap-2 text-xs font-medium ${statusDot}`}
      >
        <span className="nk-status-dot" />
        {statusLabel}
      </div>

      {detectionError && (
        <p
          role="alert"
          className="nk-alert-danger mt-3"
        >
          {detectionError}
        </p>
      )}

      <div className="mt-5 grid grid-cols-1 gap-2 sm:grid-cols-2">
        {targets.map((target) => {
          const renderState = getTargetRenderState(detectionStatus, target.installed);
          const selected = renderState === "missing" && selectedTarget?.id === target.id;
          return (
            <button
              key={target.id}
              type="button"
              onClick={() => setSelectedTargetId(target.id)}
              disabled={renderState !== "missing"}
              aria-pressed={selected}
              className={`nk-row min-w-0 text-left transition ${
                selected
                  ? "nk-row-selected"
                  : renderState === "installed"
                    ? "bg-[var(--nk-success-soft)]"
                    : ""
              }`}
            >
              <div className="flex min-w-0 items-center gap-3">
                <TargetAppIcon
                  targetId={target.id}
                  name={target.name}
                  icon={target.icon}
                  size="md"
                />
                <div className="min-w-0 flex-1">
                  <h3 className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">
                    {target.name}
                  </h3>
                  {renderState === "installed" ? (
                    <p className="mt-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
                      已安装 · 准备就绪
                    </p>
                  ) : renderState === "missing" ? (
                    <p className="mt-0.5 text-xs font-medium text-amber-600 dark:text-amber-400">
                      还没有安装
                    </p>
                  ) : renderState === "checking" ? (
                    <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">
                      正在读取安装状态…
                    </p>
                  ) : (
                    <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">
                      暂未获得安装状态
                    </p>
                  )}
                </div>
              </div>
            </button>
          );
        })}
      </div>

      {actionError && (
        <p
          role="alert"
          className="nk-alert-warning mt-3 break-words"
        >
          {actionError}
        </p>
      )}

      {detectionStatus === "checking" && (
        <div className="mt-6 flex min-h-72 flex-col items-center justify-center text-center">
          <span className="nk-spinner" aria-hidden="true" />
          <p className="mt-4 text-sm font-medium text-gray-800 dark:text-gray-200">正在查找已安装的应用</p>
          <p className="mt-1 max-w-sm text-xs leading-5 text-gray-500 dark:text-gray-400">
            通常只需要几秒钟，请稍候。
          </p>
        </div>
      )}

      {detectionStatus === "error" && (
        <div className="mt-6 flex min-h-72 flex-col items-center justify-center text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-[var(--nk-danger-soft)] text-[var(--nk-danger)]">
            <RefreshIcon />
          </div>
          <p className="mt-4 text-sm font-medium text-gray-800 dark:text-gray-200">暂时没能读取安装状态</p>
          <p className="mt-1 max-w-sm text-xs leading-5 text-gray-500 dark:text-gray-400">
            确认应用已经安装后，再点击右上角“重新检查”。
          </p>
        </div>
      )}

      {showGuide && selectedTarget && (
        <div className="mt-6 border-t pt-6 [border-color:var(--nk-line)]">
          <div className="flex items-center gap-4">
            <TargetAppIcon
              targetId={selectedTarget.id}
              name={selectedTarget.name}
              icon={selectedTarget.icon}
              size="lg"
            />
            <div className="min-w-0">
              <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">
                安装 {selectedTarget.name}
              </h3>
              <p className="mt-1 text-sm leading-6 text-gray-500 dark:text-gray-400">
                当前设备是 {platform}。整个过程通常只需几分钟，安装完成后无需关闭 Niko。
              </p>
            </div>
          </div>

          <ol className="mt-6 grid grid-cols-1 gap-5 sm:grid-cols-2 min-[1120px]:grid-cols-4">
            {guideSteps.map((step, index) => (
              <li
                key={step.title}
                className="min-w-0 border-l-2 pl-4 [border-color:var(--nk-line)]"
              >
                <div className={`flex h-10 w-10 items-center justify-center rounded-xl ${step.tone}`}>
                  <InstallGuideIcon name={step.icon} />
                </div>
                <p className="mt-3 text-xs font-semibold text-gray-500 dark:text-gray-400">
                  第 {index + 1} 步
                </p>
                <h4 className="mt-1 text-sm font-semibold text-gray-900 dark:text-gray-100">
                  {step.title}
                </h4>
                <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-gray-400">
                  {step.description}
                </p>
              </li>
            ))}
          </ol>

          <div className="mt-6 flex flex-col gap-3 border-t pt-4 [border-color:var(--nk-line)] sm:flex-row sm:items-center sm:justify-between">
            <p className="text-xs leading-5 text-gray-500 dark:text-gray-400">
              下载与安装都在 {getTargetShortName(selectedTarget)} 官方页面完成。
            </p>
            <button
              type="button"
              onClick={() => onOpenInstaller(selectedTarget)}
              className="nk-btn-primary w-full sm:w-auto"
            >
              打开 {getTargetShortName(selectedTarget)} 官方下载页
              <ExternalLinkIcon />
            </button>
          </div>
        </div>
      )}

      {allInstalled && (
        <div className="mt-6 flex min-h-72 flex-col items-center justify-center text-center">
          <div className="flex items-center gap-3">
            {targets.map((target) => (
              <TargetAppIcon
                key={target.id}
                targetId={target.id}
                name={target.name}
                icon={target.icon}
                size="lg"
              />
            ))}
          </div>
          <p className="mt-5 text-base font-semibold text-gray-900 dark:text-gray-100">应用已经准备好了</p>
          <p className="mt-1 max-w-md text-sm leading-6 text-gray-500 dark:text-gray-400">
            现在可以在右侧登录 Niko。登录后选择应用和模型，即可完成接入。
          </p>
        </div>
      )}
      </div>
    </section>
  );
}

export default function Login() {
  const navigate = useNavigate();
  const [stage, setStage] = useState<Stage>("login");
  const [pendingToken, setPendingToken] = useState("");

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [registrationUsername, setRegistrationUsername] = useState("");
  const [registrationPassword, setRegistrationPassword] = useState("");
  const [passwordConfirmation, setPasswordConfirmation] = useState("");
  const [verificationNonce, setVerificationNonce] = useState("");
  const [verificationState, setVerificationState] = useState<VerificationState>("idle");
  const verificationNonceRef = useRef("");
  const registrationGate = useRef(createRegistrationSubmissionGate()).current;

  const [remember, setRemember] = useState(false);
  const [credentialSource, setCredentialSource] = useState<CredentialSource>("login");

  const [targets, setTargets] = useState<LoginTarget[]>(() => mapLoginTargets([]));
  const [detectionStatus, setDetectionStatus] = useState<DetectionStatus>("checking");
  const [detectionError, setDetectionError] = useState("");
  const [targetActionError, setTargetActionError] = useState("");

  // 设备数达上限：登录页当场列出旧设备供用户勾选退出，避免被挡在门外
  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [deviceLimit, setDeviceLimit] = useState(0);
  const [selectedDevices, setSelectedDevices] = useState<number[]>([]);
  // 记住触发上限时处于哪一步，释放设备后按原路重试
  const [limitFrom, setLimitFrom] = useState<LoginStage>("login");

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

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

  const detectTargets = async () => {
    setDetectionStatus("checking");
    setDetectionError("");
    setTargetActionError("");
    setTargets(mapLoginTargets([]));
    try {
      const list = await invoke<TargetInfo[]>("list_targets");
      setTargets(mapLoginTargets(list));
      setDetectionStatus("success");
    } catch {
      setDetectionStatus("error");
      setDetectionError("未能读取本机应用状态，请重新检查。");
    }
  };

  useEffect(() => {
    void detectTargets();
  }, []);

  useEffect(() => {
    verificationNonceRef.current = verificationNonce;
  }, [verificationNonce]);

  useEffect(() => () => {
    const nonce = verificationNonceRef.current;
    if (nonce) void invoke("cancel_registration_challenge", { nonce });
  }, []);

  useEffect(() => {
    if (stage !== "register" || !verificationNonce || verificationState !== "pending") return;
    let stopped = false;
    const poll = async () => {
      try {
        const result = await invoke<ChallengeStatus>("registration_challenge_status", {
          nonce: verificationNonce,
        });
        if (stopped) return;
        if (result.status === "verified") {
          setVerificationState("verified");
          setError("");
        } else if (result.status === "expired") {
          setVerificationNonce("");
          setVerificationState("idle");
          setError("安全验证已过期，请重新完成验证");
        } else if (result.status === "missing") {
          setVerificationNonce("");
          setVerificationState("idle");
          setError("安全验证窗口已关闭，请重新完成验证");
        }
      } catch {
        if (!stopped) {
          setVerificationNonce("");
          setVerificationState("idle");
          setError("未能读取安全验证状态，请重新验证");
        }
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), 400);
    return () => {
      stopped = true;
      window.clearInterval(interval);
    };
  }, [stage, verificationNonce, verificationState]);

  const handleOpenInstaller = async (target: LoginTarget) => {
    setTargetActionError("");
    try {
      await open(target.downloadUrl);
    } catch {
      setTargetActionError("无法打开下载页面，请手动打开浏览器访问官方页面。");
    }
  };

  const handleLogin = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password.trim()) { setError("请填写账号和密码"); return; }
    setCredentialSource("login");
    setError(""); setNotice(""); setLoading(true);
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
      await finishLogin(result.access_token!, result.username ?? username, true);
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, "login");
        return;
      }
      setError(friendlyLoginError(err));
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
      await finishLogin(
        result.access_token!,
        result.username ?? username,
        credentialSource === "login",
      );
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, "2fa");
        return;
      }
      setError(friendlyLoginError(err));
    } finally {
      setLoading(false);
    }
  };

  const enterDeviceLimit = (err: DeviceLimitError, from: LoginStage) => {
    setDevices(err.devices);
    setDeviceLimit(err.deviceLimit);
    setSelectedDevices([]);
    setLimitFrom(from);
    setError(friendlyLoginError(err));
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
      await finishLogin(
        result.access_token!,
        result.username ?? username,
        credentialSource === "login",
      );
    } catch (err) {
      if (err instanceof DeviceLimitError) {
        enterDeviceLimit(err, limitFrom);
        return;
      }
      setError(friendlyLoginError(err));
    } finally {
      setLoading(false);
    }
  };

  const finishLogin = async (token: string, uname: string, updateRememberedLogin: boolean) => {
    const rememberSession = shouldPersistAuthSession(updateRememberedLogin, remember);
    // 记住我：凭证写入系统钥匙串；未勾选则清掉历史记录
    if (updateRememberedLogin) {
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
    }
    try {
      const [bootstrap, status] = await Promise.all([
        api.bootstrap(token),
        api.status().catch(() => null),
      ]);
      const provision = await api.provision(token, bootstrap.user.group);
      const balance = parseBalanceSnapshot(
        bootstrap.user.quota,
        bootstrap.site.quota_per_unit ?? status?.quota_per_unit,
      );
      saveAuth({
        accessToken: token,
        username: uname,
        userId: bootstrap.user.id,
        quota: bootstrap.user.quota,
        quotaPerUnit: balance?.quotaPerUnit,
        balanceUpdatedAt: balance?.updatedAt,
        defaultGroup: bootstrap.user.group,
        apiKey: provision.api_key,
        remember: rememberSession,
      });
      navigate("/home");
    } catch {
      // bootstrap/provision 失败不阻断登录，仍跳首页
      saveAuth({
        accessToken: token,
        username: uname,
        userId: 0,
        quota: 0,
        defaultGroup: "",
        apiKey: "",
        remember: rememberSession,
      });
      navigate("/home");
    }
  };

  const selectAuthMode = (mode: AuthMode) => {
    const currentMode: AuthMode = stage === "register" ? "register" : "login";
    if (mode === currentMode || !["login", "register"].includes(stage)) return;
    if (currentMode === "register") {
      const nonce = verificationNonce;
      if (nonce) void invoke("cancel_registration_challenge", { nonce });
      setVerificationNonce("");
      setVerificationState("idle");
      setRegistrationPassword("");
      setPasswordConfirmation("");
    }
    setStage(toggleAuthMode(currentMode));
    setError("");
    setNotice("");
  };

  const beginRegistrationVerification = async () => {
    if (loading || verificationState === "opening") return;
    setError("");
    setNotice("");
    setVerificationState("opening");
    try {
      const challenge = await invoke<ChallengeStart>("start_registration_challenge");
      setVerificationNonce(challenge.nonce);
      setVerificationState("pending");
    } catch (err) {
      setVerificationNonce("");
      setVerificationState("idle");
      setError(registrationErrorMessage(err));
    }
  };

  const handleRegistration = async (event: React.FormEvent) => {
    event.preventDefault();
    const fields = {
      username: registrationUsername,
      password: registrationPassword,
      passwordConfirmation,
    };
    const validationError = validateRegistration(fields);
    if (validationError) { setError(validationError); return; }
    if (!verificationNonce || verificationState !== "verified") {
      setError("请先完成安全验证");
      return;
    }

    const registeredUsername = registrationUsername.trim();
    const registeredPassword = registrationPassword;
    let started = false;
    try {
      const submission = await registrationGate.run(async () => {
        started = true;
        setError("");
        setNotice("");
        setLoading(true);
        return registerThenLogin(
          registeredUsername,
          () => invoke("register_niko_account", {
            request: {
              nonce: verificationNonce,
              username: registeredUsername,
              password: registeredPassword,
            },
          }),
          () => api.login({
            username: registeredUsername,
            password: registeredPassword,
            deviceId: getDeviceId(),
            deviceName: getDeviceName(),
            platform: getDeviceName(),
          }),
        );
      });
      if (!submission.started) return;

      setVerificationNonce("");
      setVerificationState("idle");
      setRegistrationPassword("");
      setPasswordConfirmation("");
      setUsername(registeredUsername);
      setRemember(false);
      setCredentialSource("registration");

      const outcome = submission.value;
      if (outcome.kind === "created") {
        setNotice(outcome.notice);
        if (outcome.loginError instanceof DeviceLimitError) {
          setPassword(registeredPassword);
          enterDeviceLimit(outcome.loginError, "login");
          return;
        }
        setPassword("");
        setStage("login");
        setError("自动登录未完成，请输入密码继续登录");
        return;
      }

      const result = outcome.login;
      if (result.require_2fa && result.pending_token) {
        setPassword(registeredPassword);
        setPendingToken(result.pending_token);
        setStage("2fa");
        return;
      }
      await finishLogin(result.access_token!, result.username ?? registeredUsername, false);
    } catch (err) {
      setVerificationNonce("");
      setVerificationState("idle");
      setError(registrationErrorMessage(err));
    } finally {
      if (started) setLoading(false);
    }
  };

  return (
    <main className="min-h-screen overflow-y-auto md:h-screen md:overflow-hidden">
      <div className="grid min-h-screen w-full md:h-full md:min-h-0 md:grid-cols-[minmax(0,1fr)_minmax(320px,360px)]">
        <TargetPreparation
          targets={targets}
          detectionStatus={detectionStatus}
          detectionError={detectionError}
          actionError={targetActionError}
          onDetect={() => void detectTargets()}
          onOpenInstaller={(target) => void handleOpenInstaller(target)}
        />

        <section
          aria-labelledby="auth-title"
          className="flex min-w-0 flex-col border-t bg-white/55 p-6 [border-color:var(--nk-line)] dark:bg-white/[0.025] sm:p-8 md:min-h-0 md:overflow-y-auto md:border-l md:border-t-0"
        >
          <div className="mx-auto my-auto w-full max-w-sm">
        <div className="mb-7 text-center">
          <div className="mb-3 flex justify-center">
            <Logo size={56} />
          </div>
          <h1 id="auth-title" className="sr-only">{BRAND.name}</h1>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            {stage === "login"
              ? BRAND.tagline
              : stage === "register"
                ? "创建 Niko 账号"
              : stage === "2fa"
                ? "请输入两步验证码"
                : "选择要退出登录的设备"}
          </p>
        </div>

        {(stage === "login" || stage === "register") && (
          <div
            role="tablist"
            aria-label="账户入口"
            className="mb-5 grid grid-cols-2 rounded-md bg-[var(--nk-surface-muted)] p-1"
          >
            {(["login", "register"] as const).map((mode) => {
              const selected = stage === mode;
              return (
                <button
                  key={mode}
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  onClick={() => selectAuthMode(mode)}
                  disabled={loading}
                  className={`rounded px-3 py-1.5 text-xs font-medium transition ${
                    selected
                      ? "bg-[var(--nk-surface)] text-gray-900 shadow-sm dark:text-gray-100"
                      : "text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200"
                  }`}
                >
                  {mode === "login" ? "登录" : "注册"}
                </button>
              );
            })}
          </div>
        )}

        {notice && (
          <div role="status" className="nk-alert-success mb-4 text-sm">
            {notice}
          </div>
        )}

        {error && (
          <div
            role="alert"
            className="nk-alert-danger mb-4 text-sm"
          >
            {error}
          </div>
        )}

        {stage === "device-limit" ? (
          <div className="space-y-4">
            <p className="text-xs text-gray-500 dark:text-gray-400">
              已登录 {devices.length}
              {deviceLimit > 0 && ` / ${deviceLimit}`} 台。勾选不再使用的设备，退出后即可继续登录。
            </p>
            <div className="max-h-44 space-y-2 overflow-y-auto">
              {devices.map((d) => {
                const checked = selectedDevices.includes(d.id);
                return (
                  <label
                    key={d.id}
                    className="nk-row flex cursor-pointer items-center gap-3"
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
                        {displayDeviceLabel(d.device_name, d.platform)}
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
              className="nk-btn-primary w-full py-2 text-sm"
            >
              {loading
                ? "处理中…"
                : `退出所选 ${selectedDevices.length} 台并登录`}
            </button>
            <button
              type="button"
              onClick={() => { setStage(limitFrom); setError(""); }}
              disabled={loading}
              className="nk-btn-ghost w-full text-sm"
            >
              ← 返回
            </button>
          </div>
        ) : stage === "login" ? (
          <form onSubmit={handleLogin} className="space-y-4">
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">账号</label>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="nk-input w-full"
                placeholder="用户名或邮箱"
                autoComplete="username"
                disabled={loading}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">密码</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="nk-input w-full"
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
              className="nk-btn-primary w-full py-2 text-sm"
            >
              {loading ? "登录中…" : "登录"}
            </button>

          </form>
        ) : stage === "register" ? (
          <form onSubmit={handleRegistration} className="space-y-4">
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">用户名</label>
              <input
                type="text"
                value={registrationUsername}
                onChange={(event) => setRegistrationUsername(event.target.value)}
                className="nk-input w-full"
                placeholder="2-20 个字符"
                autoComplete="username"
                minLength={2}
                maxLength={20}
                disabled={loading}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">密码</label>
              <input
                type="password"
                value={registrationPassword}
                onChange={(event) => setRegistrationPassword(event.target.value)}
                className="nk-input w-full"
                placeholder="8-20 个字符"
                autoComplete="new-password"
                minLength={8}
                maxLength={20}
                disabled={loading}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">确认密码</label>
              <input
                type="password"
                value={passwordConfirmation}
                onChange={(event) => setPasswordConfirmation(event.target.value)}
                className="nk-input w-full"
                placeholder="再次输入密码"
                autoComplete="new-password"
                minLength={8}
                maxLength={20}
                disabled={loading}
              />
            </div>

            <div className="rounded-md border p-3 [border-color:var(--nk-line)]">
              <div className="flex min-h-8 items-center justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-xs font-medium text-gray-800 dark:text-gray-200">安全验证</p>
                  <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400" aria-live="polite">
                    {verificationState === "verified"
                      ? "已完成"
                      : verificationState === "pending"
                        ? "验证窗口已打开"
                        : verificationState === "opening"
                          ? "正在打开"
                          : "尚未完成"}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => void beginRegistrationVerification()}
                  disabled={loading || ["opening", "pending"].includes(verificationState)}
                  className="nk-btn-secondary shrink-0 px-3 py-1.5 text-xs"
                >
                  {verificationState === "verified" ? "重新验证" : "开始验证"}
                </button>
              </div>
            </div>

            <button
              type="submit"
              disabled={loading}
              className="nk-btn-primary w-full py-2 text-sm"
            >
              {loading ? "正在创建…" : "创建账号"}
            </button>
          </form>
        ) : (
          <form onSubmit={handle2FA} className="space-y-4">
            <div>
              <label className="mb-1 block text-xs text-gray-500 dark:text-gray-400">6 位验证码</label>
              <input
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                maxLength={6}
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className="nk-input w-full text-center text-lg"
                placeholder="000000"
                autoFocus
                disabled={loading}
              />
            </div>
            <button
              type="submit"
              disabled={loading}
              className="nk-btn-primary w-full py-2 text-sm"
            >
              {loading ? "验证中…" : "确认"}
            </button>
            <button
              type="button"
              onClick={() => { setStage("login"); setCode(""); setError(""); }}
              className="nk-btn-ghost w-full text-sm"
            >
              ← 返回登录
            </button>
          </form>
        )}
        <button
          type="button"
          onClick={() => navigate("/sessions")}
          className="nk-btn-ghost mt-5 w-full text-sm"
        >
          <BookOpenIcon />
          查看 ChatGPT 会话
        </button>
          </div>
        </section>
      </div>
    </main>
  );
}
