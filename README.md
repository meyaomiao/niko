# momo·摸摸 Launcher

momotoken 桌面登录器 — 登录账号后一键将 momotoken API 接入 Codex、Claude Desktop 等工具，无需手动填写 URL 或 API Key。

## 技术栈

- **前端**: React 19 + TypeScript + Tailwind CSS (Vite)
- **后端**: Tauri 2 (Rust)
- **平台**: macOS 12+，Windows 10+（首版未签名）

## 本地开发

> 需要安装 [Rust stable](https://rustup.rs/) 和 Node.js 20+

```bash
cd launcher
npm install
npm run tauri dev
```

## 构建

```bash
npm run tauri build
```

## 目录结构

```
launcher/
├── src/                   # React 前端
│   ├── pages/             # 页面组件
│   ├── store/             # 状态管理（Zustand，E3-x 实现）
│   ├── api/               # HTTP 客户端封装
│   └── i18n/              # 前端国际化
└── src-tauri/             # Tauri / Rust 后端
    └── src/
        ├── commands/      # Tauri commands（IPC 入口）
        ├── targets/       # Target trait 与各接入目标实现
        ├── fsx/           # 原子写与快照回滚
        └── credentials/  # Keychain 凭证存储封装
```

## Issue 路线图

| Epic | 内容 |
|------|------|
| E2 | 工程基础：Tauri 框架、fsx、凭证存储 |
| E3 | 认证：Turnstile + 登录/2FA/会话续期 |
| E4 | 主界面：额度卡片、模型选择、用量明细 |
| E5 | 配置写入：TOML/JSON 保守合并、漂移检测 |
| E6 | 目标接入：Codex、Claude Desktop、Claude Code CLI |
| E9 | 打包发布：macOS 签名公证、Windows 产物 |
