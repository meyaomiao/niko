export type DesktopErrorKind =
  | "session"
  | "balance"
  | "network"
  | "service"
  | "not_installed"
  | "not_configured"
  | "generic";

function errorParts(value: unknown): { code: string; message: string } {
  if (typeof value === "string") return { code: "", message: value };
  if (value && typeof value === "object") {
    const candidate = value as Record<string, unknown>;
    return {
      code: typeof candidate.code === "string" ? candidate.code : "",
      message: typeof candidate.message === "string" ? candidate.message : "",
    };
  }
  return { code: "", message: "" };
}

export function classifyDesktopError(value: unknown): DesktopErrorKind {
  const { code, message } = errorParts(value);
  const text = `${code} ${message}`.toLowerCase();
  if (/401|403|unauthor|登录.*失效|未登录|过期|session|access.?token/.test(text)) return "session";
  if (/余额|quota|insufficient|balance/.test(text)) return "balance";
  if (/未安装|找不到.*应用|未找到|no such file|not found/.test(text)) return "not_installed";
  if (/尚未接入|未启用|缺少.*配置|没有默认模型|配置.*生效|请先点击启用|请先接入/.test(text)) {
    return "not_configured";
  }
  if (/network|连接|超时|timeout|fetch|dns|offline|代理/.test(text)) return "network";
  if (/429|5\d\d|server|服务端|上游|temporarily unavailable/.test(text)) return "service";
  return "generic";
}

export function friendlyDesktopError(value: unknown): string {
  switch (classifyDesktopError(value)) {
    case "session":
      return "登录状态已过期，请重新登录后再试。";
    case "balance":
      return "余额不足，请先充值后再试。";
    case "not_installed":
      return "还没有找到目标应用，请先安装并打开它，再试一次。";
    case "not_configured":
      return "接入还没有生效，请先接入到应用，再试一次。";
    case "network":
      return "网络连接失败，请检查网络后重试。";
    case "service":
      return "模型服务暂时不可用，请稍后重试。";
    default:
      return "操作没有完成，原有设置保持不变，请重试。";
  }
}

export function friendlyLoginError(value: unknown): string {
  const { code, message } = errorParts(value);
  const text = `${code} ${message}`.toLowerCase();
  if (/设备.*上限|device.?limit|too many devices/.test(text)) {
    return "登录设备已达到上限，请退出不再使用的设备后再试。";
  }
  if (/验证码|2fa|two.?factor|verification code/.test(text)) {
    return "验证码不正确或已过期，请重新输入后再试。";
  }
  if (/账号|用户名|密码|username|password|credential|invalid.?login|invalid.?credentials|user\s+(?:has\s+been\s+)?banned/.test(text)) {
    return "账号或密码不正确，请检查后再试。";
  }
  return friendlyDesktopError(value);
}

export function friendlyConnectivityDetail(value: unknown): string {
  const { message } = errorParts(value);
  const text = message.toLowerCase();
  if (/配置里没有默认模型|先点击启用|接入|未找到.*配置|缺少.*配置/.test(text)) {
    return "接入还没有生效，请先接入到应用后再检查。";
  }
  if (/401|403|key|密钥|权限|unauthor/.test(text)) {
    return "连接密钥无效或已过期，请重新接入后再试。";
  }
  if (/404|地址|模型不存在|default model/.test(text)) {
    return "服务地址或模型暂时不可用，请重新接入后再试。";
  }
  if (/429|频繁|rate/.test(text)) return "服务暂时繁忙，请稍后重试。";
  if (/timeout|超时|connect|连接|network|网络|代理/.test(text)) {
    return "网络连接失败，请检查网络后重试。";
  }
  return "检查没有完成，请重新接入后再试。";
}

export function displayDeviceLabel(name: string, platform: string): string {
  const cleanName = name.trim();
  if (cleanName) return cleanName;
  const cleanPlatform = platform.trim();
  return cleanPlatform ? `${cleanPlatform} 设备` : "其他设备";
}
