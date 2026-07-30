export const DESKTOP_TURNSTILE_ACTION = "niko_register";

export function validDesktopNonce(value) {
  return typeof value === "string" && /^[a-f0-9]{32}$/.test(value);
}

export function validDesktopSiteKey(value) {
  return typeof value === "string" && /^[A-Za-z0-9_-]{20,128}$/.test(value);
}

export function verificationCallbackUrl(nonce, siteKey, token) {
  if (
    !validDesktopNonce(nonce) ||
    !validDesktopSiteKey(siteKey) ||
    typeof token !== "string" ||
    token.length < 20 ||
    token.length > 4096 ||
    /[\u0000-\u001f\u007f]/.test(token)
  ) {
    throw new TypeError("Invalid desktop verification callback");
  }
  const query = new URLSearchParams({ nonce, site_key: siteKey, token });
  return `niko-register://verified?${query}`;
}

async function waitForTurnstile() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (window.turnstile?.render) return window.turnstile;
    await new Promise((resolve) => window.setTimeout(resolve, 100));
  }
  throw new Error("Turnstile failed to load");
}

async function initializeDesktopVerification() {
  const status = document.querySelector("[data-verification-status]");
  const container = document.querySelector("[data-turnstile-container]");
  const params = new URLSearchParams(window.location.search);
  const nonce = params.get("nonce") || "";
  const siteKey = params.get("site_key") || "";
  const setStatus = (message, tone = "error") => {
    status.textContent = message;
    status.dataset.tone = message ? tone : "";
  };

  if (!validDesktopNonce(nonce) || !validDesktopSiteKey(siteKey)) {
    container.hidden = true;
    setStatus("验证请求无效，请返回 Niko 重试。");
    return;
  }

  try {
    const turnstile = await waitForTurnstile();
    container.replaceChildren();
    turnstile.render(container, {
      sitekey: siteKey,
      action: DESKTOP_TURNSTILE_ACTION,
      theme: "light",
      size: "flexible",
      callback: (token) => {
        setStatus("验证完成", "success");
        window.location.replace(verificationCallbackUrl(nonce, siteKey, token));
      },
      "expired-callback": () => setStatus("验证已过期，请重新完成验证。"),
      "error-callback": () => setStatus("安全验证加载失败，请稍后重试。"),
    });
  } catch {
    container.hidden = true;
    setStatus("安全验证暂时不可用，请返回 Niko 重试。");
  }
}

if (typeof document !== "undefined" && document.body?.dataset.desktopVerification === "true") {
  void initializeDesktopVerification();
}
