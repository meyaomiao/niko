import { useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import Logo from "../components/Logo";
import { UpdateIcon } from "../components/Icons";
import {
  downloadPercent,
  friendlyUpdateError,
  installAvailableUpdate,
} from "../lib/appUpdate";
import { desktopUpdaterAdapter } from "../lib/appUpdateRuntime";

interface ForceUpgradeProps {
  currentVersion: string;
  minVersion: string;
  downloadUrl: string;
  announcement?: string;
}

export default function ForceUpgrade({
  currentVersion,
  minVersion,
  downloadUrl,
  announcement,
}: ForceUpgradeProps) {
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [percent, setPercent] = useState<number | null>(null);

  const openWebsite = async () => {
    try {
      await open(downloadUrl);
    } catch {
      window.location.href = downloadUrl;
    }
  };

  const handleUpdate = async () => {
    setBusy(true);
    setStatus(null);
    setPercent(null);
    try {
      const outcome = await installAvailableUpdate(
        await desktopUpdaterAdapter(),
        setStatus,
        (progress) => setPercent(downloadPercent(progress)),
      );
      if (outcome === "current") {
        setStatus(`请升级至 v${minVersion} 或以上。可到官网下载安装包。`);
      }
    } catch (error) {
      setStatus(friendlyUpdateError(error));
      await openWebsite();
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-6 py-8">
      <div className="nk-card w-full max-w-sm space-y-6 text-center">
        <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-[var(--nk-info-soft)] text-[var(--nk-info)]">
          <UpdateIcon className="h-7 w-7" />
        </div>

        <div>
          <h1 className="text-xl font-semibold text-gray-900 dark:text-white">需要更新</h1>
          <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
            当前版本 <span className="text-gray-900 dark:text-white">v{currentVersion}</span> 已过旧，
            请升级至 <span className="text-gray-900 dark:text-white">v{minVersion}</span> 或以上版本继续使用。
          </p>
        </div>

        {announcement && (
          <div className="nk-inset p-4 text-left">
            <p className="mb-1 text-xs font-medium uppercase text-gray-500 dark:text-gray-400">
              更新说明
            </p>
            <p className="text-sm text-gray-700 dark:text-gray-300">{announcement}</p>
          </div>
        )}

        <button
          onClick={() => void handleUpdate()}
          disabled={busy}
          className="nk-btn-primary w-full py-3 text-sm"
        >
          {busy ? "正在更新…" : "立即更新"}
        </button>
        {status && (
          <p className="nk-muted" role="status">{status}</p>
        )}
        {percent !== null && (
          <div
            className="h-1.5 overflow-hidden rounded-full bg-black/[0.06] dark:bg-white/10"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
            aria-label="更新下载进度"
          >
            <div
              className="h-full rounded-full bg-indigo-600 transition-[width] duration-200"
              style={{ width: `${percent}%` }}
            />
          </div>
        )}
        <button
          type="button"
          onClick={() => void openWebsite()}
          className="nk-btn-ghost w-full"
        >
          到官网下载
        </button>

        <div className="flex items-center justify-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <Logo size={24} />
          <span>v{currentVersion}</span>
        </div>
      </div>
    </main>
  );
}
