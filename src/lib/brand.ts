import { APP_VERSION } from "./version";

// 登录器独立品牌：与中转站 momotoken 分离，改名只动这里
export const BRAND = {
  name: "Niko",
  fullName: "Niko 登录器",
  tagline: "一键接入 AI 助手",
  version: APP_VERSION,
  siteUrl: "https://niko-ai.cc",
} as const;
