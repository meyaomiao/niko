const STATS_PATH = "/api/presence/stats";

function $(selector, root = document) {
  return root.querySelector(selector);
}

function setNotice(message) {
  const notice = $("[data-ops-notice]");
  if (notice) {
    notice.textContent = message || "";
  }
}

function formatTime(value) {
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

export function usageCopy(stats) {
  const online = stats?.online || {};
  const downloads = stats?.downloads || {};
  const versions = Object.entries(online.by_version || {})
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .map(([version, count]) => `v${version} ${count} 台`);
  return {
    onlineTotal: String(online.total ?? 0),
    downloadTotal: String(downloads.installers ?? 0),
    window: `${Math.round((stats?.window_seconds ?? 180) / 60)} 分钟`,
    generatedAt: formatTime(stats?.generated_at),
    onlineBreakdown: `macOS ${online.macos ?? 0} · Windows ${online.windows ?? 0} · Linux ${online.linux ?? 0}`,
    downloadBreakdown: `macOS ${downloads.macos ?? 0} · Windows ${downloads.windows ?? 0}，不含自动更新文件`,
    versions,
    releases: Array.isArray(downloads.by_release) ? downloads.by_release : [],
  };
}

function renderStats(stats) {
  const copy = usageCopy(stats);
  $("[data-online-total]").textContent = copy.onlineTotal;
  $("[data-download-total]").textContent = copy.downloadTotal;
  $("[data-window]").textContent = copy.window;
  $("[data-generated-at]").textContent = copy.generatedAt;
  $("[data-online-breakdown]").textContent = copy.onlineBreakdown;
  $("[data-download-breakdown]").textContent = copy.downloadBreakdown;

  const versionList = $("[data-online-versions]");
  versionList.replaceChildren();
  if (copy.versions.length === 0) {
    const empty = document.createElement("li");
    empty.className = "ops-empty";
    empty.textContent = "最近三分钟内没有收到心跳。";
    versionList.append(empty);
  } else {
    for (const line of copy.versions) {
      const item = document.createElement("li");
      item.textContent = line;
      versionList.append(item);
    }
  }

  const releaseList = $("[data-download-releases]");
  releaseList.replaceChildren();
  if (copy.releases.length === 0) {
    const empty = document.createElement("p");
    empty.className = "ops-empty";
    empty.textContent = "暂时没有安装包下载记录。";
    releaseList.append(empty);
    return;
  }
  for (const release of copy.releases) {
    const card = document.createElement("article");
    const tag = document.createElement("span");
    tag.textContent = release.tag;
    const count = document.createElement("strong");
    count.textContent = String(release.installers);
    const help = document.createElement("p");
    help.textContent = `macOS ${release.macos} · Windows ${release.windows}`;
    card.append(tag, count, help);
    releaseList.append(card);
  }
}

async function loadStats(secret) {
  const response = await fetch(STATS_PATH, {
    headers: {
      Authorization: `Bearer ${secret}`,
      Accept: "application/json",
    },
    cache: "no-store",
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(payload?.error?.message || "无法读取使用情况。");
  }
  return payload.data || payload;
}

function bind() {
  const form = $("[data-ops-form]");
  const gate = $("[data-ops-gate]");
  const content = $("[data-ops-content]");
  const submit = $("[data-ops-submit]");
  const refresh = $("[data-ops-refresh]");
  let secret = "";

  const run = async () => {
    setNotice("");
    submit.disabled = true;
    refresh.disabled = true;
    try {
      const stats = await loadStats(secret);
      renderStats(stats);
      gate.hidden = true;
      content.hidden = false;
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "无法读取使用情况。");
    } finally {
      submit.disabled = false;
      refresh.disabled = false;
    }
  };

  form?.addEventListener("submit", (event) => {
    event.preventDefault();
    const input = form.elements.namedItem("secret");
    secret = typeof input?.value === "string" ? input.value.trim() : "";
    if (secret.length < 16) {
      setNotice("请输入有效的访问口令。");
      return;
    }
    void run();
  });

  refresh?.addEventListener("click", () => {
    if (secret) {
      void run();
    }
  });
}

if (typeof document !== "undefined") {
  document.addEventListener("DOMContentLoaded", bind);
}
