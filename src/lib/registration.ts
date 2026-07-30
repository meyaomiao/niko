export type AuthMode = "login" | "register";

export interface RegistrationFields {
  username: string;
  password: string;
  passwordConfirmation: string;
}

export function toggleAuthMode(mode: AuthMode): AuthMode {
  return mode === "login" ? "register" : "login";
}

export function validateRegistration(fields: RegistrationFields): string {
  const username = fields.username.trim();
  if (username.length < 2 || username.length > 20) {
    return "用户名需为 2-20 个字符";
  }
  if (fields.password.length < 8 || fields.password.length > 20) {
    return "密码需为 8-20 个字符";
  }
  if (fields.password !== fields.passwordConfirmation) {
    return "两次输入的密码不一致";
  }
  return "";
}

export function createRegistrationSubmissionGate() {
  let active = false;
  return {
    async run<T>(task: () => Promise<T>): Promise<{ started: false } | { started: true; value: T }> {
      if (active) return { started: false };
      active = true;
      try {
        return { started: true, value: await task() };
      } finally {
        active = false;
      }
    },
  };
}

export type RegistrationAutoLoginResult<T> =
  | { kind: "authenticated"; login: T }
  | { kind: "created"; username: string; notice: string; loginError: unknown };

export async function registerThenLogin<T>(
  username: string,
  registerAccount: () => Promise<unknown>,
  login: () => Promise<T>,
): Promise<RegistrationAutoLoginResult<T>> {
  await registerAccount();
  try {
    return { kind: "authenticated", login: await login() };
  } catch (loginError) {
    return { kind: "created", username, notice: "账号已创建，请登录", loginError };
  }
}

export function registrationErrorMessage(error: unknown): string {
  const source = error && typeof error === "object" ? error as Record<string, unknown> : null;
  const code = typeof source?.code === "string" ? source.code : "";
  const retryAfter = typeof source?.retry_after_seconds === "number"
    ? source.retry_after_seconds
    : 0;

  if (["USERNAME_TAKEN", "ACCOUNT_CONFLICT"].includes(code)) {
    return "这个用户名已被使用，请换一个再试";
  }
  if (["TURNSTILE_FAILED", "TURNSTILE_REQUIRED", "VERIFICATION_REQUIRED"].includes(code)) {
    return "安全验证未通过，请重新验证";
  }
  if (["CSRF_REJECTED", "CHALLENGE_EXPIRED"].includes(code)) {
    return "安全验证已过期，请重新完成验证";
  }
  if (code === "RATE_LIMITED") {
    return retryAfter > 0
      ? `操作过于频繁，请在 ${retryAfter} 秒后重试`
      : "操作过于频繁，请稍后重试";
  }
  if (code === "TIMEOUT") return "注册请求超时，请检查网络后重试";
  if (code === "NETWORK_ERROR") return "网络连接失败，请检查连接后重试";
  if (["VERIFICATION_UNAVAILABLE", "INVALID_CONFIG"].includes(code)) {
    return "安全验证暂时不可用，请稍后重试";
  }
  if (code === "INVALID_REQUEST") return "注册信息格式无效，请检查后重试";
  return "注册失败，请稍后重试";
}
