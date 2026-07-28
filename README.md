<p align="center">
  <img src="src/assets/niko-mark.png" width="120" height="120" alt="Niko Logo">
</p>

<h1 align="center">Niko</h1>

<p align="center">
  <strong>一键接入 AI 助手的桌面工具</strong>
</p>

<p align="center">
  登录账号，选择应用和模型，剩下的配置交给 Niko。
</p>

<p align="center">
  <a href="https://github.com/meyaomiao/niko/releases/latest">
    <img src="https://img.shields.io/badge/%E4%B8%8B%E8%BD%BD-%E6%9C%80%E6%96%B0%E7%89%88%E6%9C%AC-2563eb?style=for-the-badge&amp;logo=github&amp;logoColor=white" alt="下载最新版本">
  </a>
  <a href="https://github.com/meyaomiao/niko">
    <img src="https://img.shields.io/badge/Star-%E6%94%B6%E8%97%8F%E9%A1%B9%E7%9B%AE-f5c542?style=for-the-badge&amp;logo=github&amp;logoColor=black" alt="Star 收藏项目">
  </a>
  <a href="https://github.com/meyaomiao/niko/issues">
    <img src="https://img.shields.io/badge/%E5%8F%8D%E9%A6%88-%E6%8F%90%E4%BA%A4%E9%97%AE%E9%A2%98-4b5563?style=for-the-badge&amp;logo=github&amp;logoColor=white" alt="提交问题">
  </a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-12%2B-000000?style=flat-square&amp;logo=apple&amp;logoColor=white" alt="macOS 12+">
  <img src="https://img.shields.io/badge/Windows-10%2B-0078d4?style=flat-square&amp;logo=windows&amp;logoColor=white" alt="Windows 10+">
  <img src="https://img.shields.io/badge/Tauri-2-24c8db?style=flat-square&amp;logo=tauri&amp;logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/React-19-61dafb?style=flat-square&amp;logo=react&amp;logoColor=111827" alt="React 19">
</p>

## 关于 Niko

Niko 是一个桌面端 AI 接入工具。登录账号后，选择应用、分组和模型，Niko 会自动把需要的配置写入 ChatGPT 桌面端或 Claude 桌面端。

用户不需要自己找配置文件，也不用手动复制 API 地址和密钥。Niko 不提供聊天界面，也不替代官方应用，实际使用仍在 ChatGPT 或 Claude 中完成。

服务端由 [momotoken](https://momotoken.win) 提供，本仓库只维护 Niko 客户端。

## 安装

前往 [Releases](https://github.com/meyaomiao/niko/releases/latest) 下载最新版本。

| 平台 | 系统要求 | 安装包 | 说明 |
| --- | --- | --- | --- |
| macOS | macOS 12 或更高版本 | `.dmg` | 支持 Apple Silicon 和 Intel，发布包已签名并公证 |
| Windows | Windows 10 或更高版本 | `.msi` / `*_setup.exe` | 当前发布包暂未做代码签名 |

Windows 首次安装时可能出现 SmartScreen 提示。点击“更多信息”，再点击“仍要运行”即可继续安装。

## 快速开始

1. 安装并打开 Niko。
2. 登录 momotoken 账号。
3. 选择要接入的应用、分组和模型。
4. 点击“启用”，等待 Niko 写入配置。
5. 按提示重启 ChatGPT 或 Claude。
6. 使用“连通性测试”确认配置已经生效。

需要退出接入时，可以直接恢复官方默认配置。

## 主要功能

- 登录 momotoken 账号，支持两步验证和登录设备管理。
- 查看余额、分组、模型、价格、兼容情况和用量明细。
- 自动检测已安装的 ChatGPT 桌面端和 Claude 桌面端。
- 将选中的分组和模型写入一个或全部已安装应用。
- 启动或重启目标应用，并测试当前配置是否可以正常请求模型。
- 写入前保存配置快照，支持手动恢复快照和恢复官方默认配置。
- 提供充值、主题切换、开机启动、托盘、日志导出和应用更新。

## 当前接入范围

### ChatGPT 桌面端

Niko 可以配置 ChatGPT 桌面端中 Codex 使用的模型和接口。如果用户有 ChatGPT 付费订阅，也可以保留原有登录状态，只让模型请求使用 momotoken。

### Claude 桌面端

Niko 可以配置 Claude 桌面端内置的 Claude Code 功能。Claude 普通聊天仍使用用户原有的 Anthropic 账号。

## 设计理念

- **简单**：把登录、选模型和写配置放在一个流程里。
- **轻量**：不在本地运行代理服务，协议转换由服务端处理。
- **少改配置**：只修改接入需要的字段，尽量保留用户原有设置。
- **可以恢复**：写入前保存配置快照，失败时回滚，也可以恢复官方默认配置。
- **减少密钥操作**：用户不需要查看或复制 API Key；“记住我”的登录信息保存在系统钥匙串中，导出的日志会隐藏敏感内容。

## 路线图

- [x] ChatGPT 桌面端接入
- [x] Claude 桌面端接入
- [x] 分组与模型选择
- [x] 用量、充值和设备管理
- [x] 配置快照、连通性测试和恢复官方默认
- [x] macOS 与 Windows 安装包
- [ ] 支持更多官方 AI 客户端和命令行工具
- [ ] 完善不同客户端版本的兼容检测
- [ ] 改进诊断、故障提示和配置恢复
- [ ] 完善 Windows 代码签名和安装体验

## 本地开发

需要提前安装：

- Node.js 20+
- Rust stable
- 当前平台对应的 [Tauri 2 开发依赖](https://v2.tauri.app/start/prerequisites/)

```bash
git clone https://github.com/meyaomiao/niko.git
cd niko
npm install
npm run tauri dev
```

构建安装包：

```bash
npm run tauri build
```

主要技术栈：Tauri 2、React 19、TypeScript、Tailwind CSS 和 Rust。

## 问题反馈

遇到安装、登录、模型接入或配置恢复问题，可以在 [Issues](https://github.com/meyaomiao/niko/issues) 中提交。请说明操作系统、Niko 版本、目标应用和问题现象，不要上传密码或完整 API Key。

如果 Niko 对你有帮助，可以点击仓库右上角的 **Star** 收藏项目。

## 关联项目

- [momotoken-new-api](https://github.com/meyaomiao/momotoken-new-api)：账号、模型、计费和协议转换服务。
