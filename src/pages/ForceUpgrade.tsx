import { open } from "@tauri-apps/plugin-shell";
import Logo from "../components/Logo";
import { UpdateIcon } from "../components/Icons";

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
  const handleDownload = async () => {
    try {
      await open(downloadUrl);
    } catch {
      window.location.href = downloadUrl;
    }
  };

  return (
    <main className="flex min-h-screen flex-col items-center justify-center px-6 py-8">
      <div className="nk-card w-full max-w-sm space-y-6 text-center">
        {/* 图标 */}
        <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-[var(--nk-info-soft)] text-[var(--nk-info)]">
          <UpdateIcon className="h-7 w-7" />
        </div>

        {/* 标题 */}
        <div>
          <h1 className="text-xl font-semibold text-gray-900 dark:text-white">需要更新</h1>
          <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
            当前版本 <span className="text-gray-900 dark:text-white">v{currentVersion}</span> 已过旧，
            请升级至 <span className="text-gray-900 dark:text-white">v{minVersion}</span> 或以上版本继续使用。
          </p>
        </div>

        {/* 更新说明（可选） */}
        {announcement && (
          <div className="nk-inset p-4 text-left">
            <p className="mb-1 text-xs font-medium uppercase text-gray-500 dark:text-gray-400">
              更新说明
            </p>
            <p className="text-sm text-gray-700 dark:text-gray-300">{announcement}</p>
          </div>
        )}

        {/* 下载按钮 */}
        <button
          onClick={handleDownload}
          className="nk-btn-primary w-full py-3 text-sm"
        >
          下载新版本
        </button>

        {/* 版本信息 */}
        <div className="flex items-center justify-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <Logo size={24} />
          <span>v{currentVersion}</span>
        </div>
      </div>
    </main>
  );
}
