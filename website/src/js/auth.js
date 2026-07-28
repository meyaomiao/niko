import {
  ApiError,
  apiRequest,
  getPublicConfig,
  safeInternalNext,
  unwrap,
} from "./api.js";

const page = document.body.dataset.authPage;
const form = document.querySelector("[data-auth-form]");
const statusElement = document.querySelector("[data-form-status]");
const submitButton = document.querySelector("[data-submit]");
const turnstileContainer = document.querySelector("[data-turnstile-container]");

let turnstileWidgetId;
let turnstileRequired = true;

function setStatus(message = "", tone = "error") {
  statusElement.textContent = message;
  statusElement.dataset.tone = message ? tone : "";
}

function setSubmitting(submitting) {
  submitButton.disabled = submitting;
  submitButton.textContent = submitting
    ? page === "register"
      ? "正在创建…"
      : "正在登录…"
    : page === "register"
      ? "创建账号"
      : "登录";
}

function togglePassword(button) {
  const input = button.closest(".input-wrap")?.querySelector("input");
  if (!input) {
    return;
  }
  const show = input.type === "password";
  input.type = show ? "text" : "password";
  button.textContent = show ? "隐藏" : "显示";
  button.setAttribute("aria-label", show ? "隐藏密码" : "显示密码");
}

function turnstileToken() {
  if (!turnstileRequired) {
    return "";
  }
  if (turnstileWidgetId === undefined || !window.turnstile) {
    return "";
  }
  return window.turnstile.getResponse(turnstileWidgetId);
}

function resetTurnstile() {
  if (turnstileWidgetId !== undefined && window.turnstile) {
    window.turnstile.reset(turnstileWidgetId);
  }
}

async function waitForTurnstile() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (window.turnstile?.render) {
      return window.turnstile;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 100));
  }
  throw new Error("Turnstile failed to load");
}

async function initializeSecurityCheck() {
  try {
    const config = await getPublicConfig();
    const siteKey =
      config.turnstile_site_key ||
      (config.turnstile && typeof config.turnstile.site_key === "string"
        ? config.turnstile.site_key
        : "");
    turnstileRequired =
      config.turnstile_required !== false && config.turnstile?.required !== false;

    if (!turnstileRequired) {
      turnstileContainer.hidden = true;
      return;
    }
    if (!siteKey) {
      throw new Error("Missing Turnstile site key");
    }

    const turnstile = await waitForTurnstile();
    turnstileContainer.replaceChildren();
    turnstileWidgetId = turnstile.render(turnstileContainer, {
      sitekey: siteKey,
      action: page === "register" ? "niko_register" : "niko_login",
      theme: "light",
      size: "flexible",
      callback: () => setStatus(),
      "expired-callback": () => setStatus("安全验证已过期，请重新完成验证。"),
      "error-callback": () => setStatus("安全验证加载失败，请刷新页面后重试。"),
    });
  } catch {
    submitButton.disabled = true;
    turnstileContainer.innerHTML =
      '<p class="form-status" data-tone="error">安全验证暂时不可用，请稍后刷新页面。</p>';
  }
}

function friendlyError(error) {
  if (!(error instanceof ApiError)) {
    return "请求失败，请稍后重试。";
  }
  if (error.status === 429 || error.code === "RATE_LIMITED") {
    return error.retryAfter > 0
      ? `操作过于频繁，请在 ${error.retryAfter} 秒后重试。`
      : "操作过于频繁，请稍后重试。";
  }
  if (["INVALID_CREDENTIALS", "AUTH_FAILED"].includes(error.code)) {
    return "账号或密码不正确。";
  }
  if (["TURNSTILE_FAILED", "TURNSTILE_REQUIRED"].includes(error.code)) {
    return "安全验证未通过，请重新验证。";
  }
  if (error.code === "USERNAME_TAKEN") {
    return "这个用户名暂不可用，请换一个再试。";
  }
  if (error.status >= 500) {
    return "账户服务暂时不可用，请稍后重试。";
  }
  return error.message;
}

async function submitLogin(formData) {
  await apiRequest("/auth/login", {
    method: "POST",
    body: {
      account: String(formData.get("account") || "").trim(),
      password: String(formData.get("password") || ""),
      turnstile_token: turnstileToken(),
    },
  });

  const params = new URLSearchParams(window.location.search);
  window.location.assign(safeInternalNext(params.get("next")));
}

async function submitRegistration(formData) {
  const password = String(formData.get("password") || "");
  const confirmation = String(formData.get("password_confirmation") || "");
  if (password !== confirmation) {
    throw new ApiError("两次输入的密码不一致。", { code: "PASSWORD_MISMATCH" });
  }

  const payload = await apiRequest("/auth/register", {
    method: "POST",
    body: {
      username: String(formData.get("username") || "").trim(),
      password,
      turnstile_token: turnstileToken(),
    },
  });
  unwrap(payload);
  setStatus("账号创建成功，正在前往登录。", "success");
  submitButton.disabled = true;
  window.setTimeout(() => window.location.assign("/login/?registered=1"), 900);
}

async function handleSubmit(event) {
  event.preventDefault();
  setStatus();

  if (!form.reportValidity()) {
    return;
  }
  if (turnstileRequired && !turnstileToken()) {
    setStatus("请先完成安全验证。");
    return;
  }

  setSubmitting(true);
  try {
    const formData = new FormData(form);
    if (page === "register") {
      await submitRegistration(formData);
    } else {
      await submitLogin(formData);
    }
  } catch (error) {
    setStatus(friendlyError(error));
    resetTurnstile();
    setSubmitting(false);
  }
}

for (const button of document.querySelectorAll("[data-password-toggle]")) {
  button.addEventListener("click", () => togglePassword(button));
}
form.addEventListener("submit", handleSubmit);

if (page === "login" && new URLSearchParams(window.location.search).get("registered") === "1") {
  setStatus("注册成功，请使用新账号登录。", "success");
}

initializeSecurityCheck();
