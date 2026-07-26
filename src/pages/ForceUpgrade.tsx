import { open } from "@tauri-apps/plugin-shell";

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
    <div className="flex h-screen flex-col items-center justify-center bg-transparent px-8">
      <div className="w-full max-w-sm space-y-6 text-center">
        {/* 图标 */}
        <div className="text-5xl">⬆️</div>

        {/* 标题 */}
        <div>
          <h1 className="text-xl font-semibold text-gray-900 dark:text-white">需要更新</h1>
          <p className="mt-2 text-sm text-gray-500 dark:text-gray-400 dark:text-gray-400">
            当前版本 <span className="text-gray-900 dark:text-white">v{currentVersion}</span> 已过旧，
            请升级至 <span className="text-gray-900 dark:text-white">v{minVersion}</span> 或以上版本继续使用。
          </p>
        </div>

        {/* 更新说明（可选） */}
        {announcement && (
          <div className="rounded-xl bg-white dark:bg-white/5 p-4 text-left">
            <p className="mb-1 text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
              更新说明
            </p>
            <p className="text-sm text-gray-700 dark:text-gray-300">{announcement}</p>
          </div>
        )}

        {/* 下载按钮 */}
        <button
          onClick={handleDownload}
          className="w-full rounded-xl bg-indigo-600 py-3 text-sm font-medium text-gray-900 dark:text-white transition hover:bg-indigo-500"
        >
          下载新版本
        </button>

        {/* 版本信息 */}
        <p className="text-xs text-gray-500 dark:text-gray-500">momotoken.win · v{currentVersion}</p>
      </div>
    </div>
  );
}
