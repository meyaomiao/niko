export const UNIVERSAL_MAC_PLATFORM = "darwin-universal";

export const REQUIRED_UPDATER_PLATFORMS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "windows-x86_64",
] as const;

const UPDATER_ARCH_KEYS = ["x86_64", "aarch64", "i686", "armv7"] as const;

export type UpdaterPlatformEntry = {
  url: string;
  signature: string;
};

export type UpdaterManifest = {
  version: string;
  notes?: string;
  pub_date?: string;
  platforms: Record<string, UpdaterPlatformEntry>;
};

export type DownloadProgressEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export type DownloadProgress = {
  received: number;
  total?: number;
};

export type UpdateCheckResult =
  | { kind: "available"; version: string; notes?: string }
  | { kind: "current" };

export type UpdaterHandle = {
  version: string;
  body?: string;
  downloadAndInstall: (
    onEvent?: (event: DownloadProgressEvent) => void,
  ) => Promise<void>;
};

export type UpdaterAdapter = {
  check: (options?: { timeout?: number }) => Promise<UpdaterHandle | null>;
  relaunch: () => Promise<void>;
};

export function isUpdaterPlatformKey(key: string): boolean {
  const dash = key.lastIndexOf("-");
  if (dash <= 0) return false;
  return (UPDATER_ARCH_KEYS as readonly string[]).includes(key.slice(dash + 1));
}

export function normalizeUpdaterPlatforms(
  platforms: Record<string, UpdaterPlatformEntry> | null | undefined,
): Record<string, UpdaterPlatformEntry> {
  const next: Record<string, UpdaterPlatformEntry> = { ...(platforms ?? {}) };
  const universal = next[UNIVERSAL_MAC_PLATFORM];
  if (universal?.url && universal.signature) {
    if (!next["darwin-aarch64"]) next["darwin-aarch64"] = universal;
    if (!next["darwin-x86_64"]) next["darwin-x86_64"] = universal;
  }
  delete next[UNIVERSAL_MAC_PLATFORM];
  return next;
}

export function rewriteUpdaterManifest(manifest: unknown): UpdaterManifest {
  if (!manifest || typeof manifest !== "object") {
    throw new Error("invalid updater manifest");
  }
  const raw = manifest as Record<string, unknown>;
  const version = typeof raw.version === "string" ? raw.version.trim() : "";
  if (!/^\d+\.\d+\.\d+/.test(version)) {
    throw new Error("invalid updater version");
  }

  const platformsRaw = raw.platforms;
  if (!platformsRaw || typeof platformsRaw !== "object") {
    throw new Error("invalid updater platforms");
  }

  const platforms: Record<string, UpdaterPlatformEntry> = {};
  for (const [key, value] of Object.entries(platformsRaw as Record<string, unknown>)) {
    if (!value || typeof value !== "object") continue;
    const entry = value as Record<string, unknown>;
    const url = typeof entry.url === "string" ? entry.url.trim() : "";
    const signature = typeof entry.signature === "string" ? entry.signature.trim() : "";
    if (!url || !signature) continue;
    platforms[key] = { url, signature };
  }

  const rewritten = normalizeUpdaterPlatforms(platforms);
  if (Object.keys(rewritten).length === 0) {
    throw new Error("invalid updater platforms");
  }

  return {
    version,
    notes: typeof raw.notes === "string" ? raw.notes : undefined,
    pub_date: typeof raw.pub_date === "string" ? raw.pub_date : undefined,
    platforms: rewritten,
  };
}

export function describeCheckResult(update: { version: string; body?: string } | null): UpdateCheckResult {
  if (!update) return { kind: "current" };
  return { kind: "available", version: update.version, notes: update.body };
}

export function accumulateDownloadProgress(
  prev: DownloadProgress,
  event: DownloadProgressEvent,
): DownloadProgress {
  if (event.event === "Started") {
    return { received: 0, total: event.data.contentLength };
  }
  if (event.event === "Progress") {
    return { received: prev.received + event.data.chunkLength, total: prev.total };
  }
  return prev;
}

export function formatDownloadProgress(received: number, total?: number): string {
  if (total && total > 0) {
    const percent = Math.min(99, Math.floor((received / total) * 100));
    return `正在下载新版本 ${percent}%`;
  }
  if (received > 0) {
    const mb = (received / (1024 * 1024)).toFixed(1);
    return `正在下载新版本 ${mb} MB`;
  }
  return "正在下载新版本…";
}

export function downloadPercent(progress: DownloadProgress): number | null {
  if (!progress.total || progress.total <= 0) return null;
  return Math.min(99, Math.floor((progress.received / progress.total) * 100));
}

function errorText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.message;
  if (value && typeof value === "object") {
    const candidate = value as Record<string, unknown>;
    if (typeof candidate.message === "string") return candidate.message;
  }
  return "";
}

export function friendlyUpdateError(value: unknown): string {
  const text = errorText(value).toLowerCase();
  if (/could not fetch a valid release json|failed to fetch.*json|invalid json/.test(text)) {
    return "暂时无法读取更新信息，请稍后重试。";
  }
  if (
    /darwin-universal|unsupported target|target .* not found|platform .* not found|no update available for/.test(text)
    || (/platform/.test(text) && /not found|missing|unsupported/.test(text))
  ) {
    return "当前系统还没有匹配的更新包，请稍后重试或到官网下载。";
  }
  if (/signature|minisign|checksum|verify/.test(text)) {
    return "更新包校验失败，没有安装。请稍后重试或到官网重新下载。";
  }
  if (/timeout|timed out|network|dns|connect|tls|certificate|proxy|offline|连接/.test(text)) {
    return "检查更新需要联网，请确认网络后重试。";
  }
  if (/403|404|429|5\d\d/.test(text)) {
    return "更新服务暂时不可用，请稍后重试。";
  }
  return "检查更新没有完成，请稍后重试。";
}

export async function installAvailableUpdate(
  adapter: UpdaterAdapter,
  onStatus: (status: string) => void,
  onProgress: (progress: DownloadProgress) => void,
): Promise<"current" | "installed"> {
  onStatus("正在检查新版本…");
  const update = await adapter.check({ timeout: 20_000 });
  const result = describeCheckResult(update);
  if (result.kind === "current" || !update) {
    onStatus("已是最新版本");
    return "current";
  }

  onStatus(`发现新版本 ${result.version}，正在下载…`);
  let progress: DownloadProgress = { received: 0 };
  await update.downloadAndInstall((event) => {
    progress = accumulateDownloadProgress(progress, event);
    onProgress(progress);
    if (event.event === "Finished") {
      onStatus("正在安装新版本…");
    } else {
      onStatus(formatDownloadProgress(progress.received, progress.total));
    }
  });

  onStatus("安装完成，即将重启…");
  try {
    await adapter.relaunch();
  } catch {
    onStatus("安装完成，请手动重启应用以使用新版本。");
  }
  return "installed";
}
