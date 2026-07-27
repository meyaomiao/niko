# Niko

一键接入 AI 助手的桌面登录器 — 登录账号后自动把中转 API 写入 ChatGPT 桌面端与 Claude 桌面端，无需手动填写 URL 或 API Key。

服务端接口由 [momotoken](https://momotoken.win) 提供（`/api/client/*`），本仓库只维护客户端。

## 技术栈

- **前端**: React 19 + TypeScript + Tailwind CSS (Vite)
- **后端**: Tauri 2 (Rust)
- **平台**: macOS 12+（签名公证）、Windows 10+（当前未签名）

## 本地开发

> 需要 [Rust stable](https://rustup.rs/) 与 Node.js 20+

```bash
npm install
npm run tauri dev
```

## 构建

```bash
npm run tauri build
```

## 目录结构

```
├── src/                   # React 前端
│   ├── pages/             # 页面组件
│   ├── store/             # 状态管理
│   ├── api/               # HTTP 客户端封装
│   └── i18n/              # 前端国际化
├── src-tauri/             # Tauri / Rust 后端
│   └── src/
│       ├── commands/      # Tauri commands（IPC 入口）
│       ├── targets/       # Target trait 与各接入目标实现
│       ├── fsx/           # 原子写与快照回滚
│       └── credentials/   # Keychain 凭证存储封装
└── docs/                  # 设计方案与产品规划
```

## 发布

推送 `niko-v*` tag 触发 [release workflow](.github/workflows/release.yml)，产出 macOS 签名公证包、Windows 未签名包与 updater 所需的 `latest.json`。

## 关联仓库

- [momotoken-new-api](https://github.com/meyaomiao/momotoken-new-api) — 中转站服务端与网页前端
