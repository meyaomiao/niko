export interface UsageReleaseDownloads {
  tag: string;
  installers: number;
  macos: number;
  windows: number;
}

export interface UsageStats {
  online: {
    total: number;
    macos: number;
    windows: number;
    linux: number;
    by_version?: Record<string, number>;
  };
  downloads: {
    installers: number;
    macos: number;
    windows: number;
    by_release?: UsageReleaseDownloads[];
  };
  generated_at: string;
  window_seconds: number;
}

export function formatUsageGeneratedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function usageSummary(stats: UsageStats) {
  const versions = Object.entries(stats.online.by_version ?? {})
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .map(([version, count]) => `v${version} ${count} 台`);
  return {
    onlineTotal: stats.online.total,
    downloadTotal: stats.downloads.installers,
    windowMinutes: Math.round((stats.window_seconds || 180) / 60),
    generatedAt: formatUsageGeneratedAt(stats.generated_at),
    onlineBreakdown: `macOS ${stats.online.macos} · Windows ${stats.online.windows} · Linux ${stats.online.linux}`,
    downloadBreakdown: `macOS ${stats.downloads.macos} · Windows ${stats.downloads.windows}，不含自动更新文件`,
    versions,
    releases: stats.downloads.by_release ?? [],
  };
}
