# momo 登录器产品方案

- 状态：草案 v3（v1「Agent 管理器」、v2「深链驱动 CC Switch」两版定位均已废弃）
- 一句话定位：momo·摸摸的官方桌面登录器。登录账号 → 看额度 → 选模型 → 一键接入官方 Agent。所有能力是自己的原生代码，不套壳、不依赖用户另装任何应用
- 归属仓库：客户端 `meyaomiao/momo-launcher`，服务端改动 `meyaomiao/momotoken-new-api`
- 关联流程：`.github/ISSUE_WORKFLOW.md`

## 1. 三个决策点（本版核心变更）

### 1.1 登录：不再走系统浏览器，改为登录器内原生登录

v2 的浏览器 + PKCE 方案被否决，理由成立：momoToken 就是账号密码体系，客户端内直接登录是最短路径。

核实服务端后确认可行，并有一个必须处理的细节：

- `POST /api/user/login` 接受用户名 + 密码（`controller/user.go:40`），返回后写 session cookie。
- 该路由挂了 `middleware.TurnstileCheck()`（`router/api-router.go:74`），而 momotoken 已启用 Turnstile，所以裸调接口会被拒。
- 若账号开了 2FA，服务端会先写 pending session 要求二次验证（`controller/user.go:76`）。
- 登录成功后调 `GET /api/user/self/token` 换访问令牌，后续接口用令牌，不必长期依赖 cookie。

解法：登录器前端本身就是 WebView（Tauri 架构），Turnstile 的官方 JS 组件可以直接渲染在登录器自己的登录页里，用户看到的是一个原生登录框加一个人机验证勾选框，全程不跳浏览器。2FA 同理在登录器内做第二步输入。这样既不绕过风控，也不牺牲原生感。

备选：新增一个专供客户端的登录接口，豁免 Turnstile 但强化限流与失败锁定。仅在 Turnstile 组件在 WebView 内表现不佳时启用。

登录后台动作：自动调用 `POST /api/token` 创建本机专属 Key（名称含设备标识），`GET /api/token/:id/key` 取回，存入 Keychain / Credential Manager。用户从头到尾看不到 Key。

### 1.2 形态：CC Switch 的实现方式作为我们自己的代码，不做产品套产品

v2 的「伴侣模式 / 深链驱动」方案被否决，理由同样成立：让用户装两个应用、还要理解它们的关系，违背登录器的存在意义。

确认采用：momo 登录器是一个独立的、自有的应用。参考 CC Switch 的实现方式（配置文件格式、写入时序、原子回滚策略），把这些做法落成 momo 自己的代码，UI、交互、品牌、代码结构全部自有。用户感知不到 CC Switch 的存在。

### 1.3 开发量：原生化不但没变大，反而比 v2 更小

这是本次核实最重要的发现。v2 之所以倾向套用 CC Switch，是因为假设自研要背上它那个巨型本地网关。核实后这个假设不成立。

CC Switch v3.18.0 体量（commit 878c26f，实测行数）：

| 部分 | 行数 | momo 是否需要 |
| --- | --- | --- |
| `proxy/providers/` 协议转换 | 35,659 | 不需要，服务端已具备（见 1.4） |
| `proxy/` 网关核心（转发、SSE、熔断、故障切换） | 约 10,050 | 不需要，同上 |
| 配置写入（codex + claude desktop + provider + 存储） | 9,301 | 需要其中一小部分 |
| WebDAV / S3 同步、会话管理、MCP、技能、其他 6 种 CLI | 约 16,000 | 完全不需要 |
| 前端 TypeScript | 83,972 | 不需要，momo 自己做 UI |

momo 真正要写的配置层还能进一步收窄：CC Switch 要支持 8 个应用 × 任意第三方供应商 × 多 Profile 切换，momo 只需要 3 个应用 × 一个固定上游（momotoken）。同样的功能，我们的代码量是它的一小部分。

结论：原生实现的增量工作量主要落在「配置文件读写 + 原子备份回滚」这一块，量级可控；反而省掉了 v2 里 fork 巨型仓库、持续跟上游同步、以及在别人代码里塞 momo 业务逻辑的长期成本。

### 1.4 关键前提：本地网关可以整体不做，因为服务端已经会转协议

核实 momotoken（new-api）代码后确认，跨协议能力在服务端就有，共 12 条已注册的转换路由（`service/relayconvert/text_converter_registry.go`）：

| 输入协议 | 可转出的上游协议 |
| --- | --- |
| Claude Messages | OpenAI Chat、Gemini、OpenAI Responses |
| OpenAI Chat | Claude、Gemini、OpenAI Responses |
| Gemini | OpenAI Chat、Claude、OpenAI Responses |
| OpenAI Responses | OpenAI Chat、Claude、Gemini |

服务端同时暴露了 Claude 协议入口 `POST /v1/messages`（`router/relay-router.go:88`）。

这意味着「Claude Desktop 用 GPT 模型」「Codex 用 Claude 模型」这类跨平台需求，只要把请求发到 momotoken 就完成了，转换发生在服务端。登录器不需要在本机跑代理进程，也就同时消掉了 v2 方案里最难的部分和最烦的用户体验问题（进程必须常驻、退出即失效、端口冲突）。

需要验证：`/v1/messages` 入口配上非 Anthropic 上游渠道时的实际转换质量，尤其是流式与工具调用（见第 9 节）。

## 2. 关于 MIT 与归属的说明

你问的「必须完整保留 MIT 许可与 Jason Young 归属」是什么意思 —— 这条只在 v2 的 fork 方案下成立，本版已不再需要按那种方式处理。

背景：CC Switch 的许可证是 MIT，版权行为 `Copyright (c) 2025 Jason Young`（作者）。MIT 允许任意商用、修改、闭源分发，几乎没有限制，但有一个硬性条件：如果你复制了它的代码，分发时必须带上原许可证全文和版权声明。v2 打算 fork 整个仓库，那就属于「复制代码」，所以必须在产品里保留那份声明，否则构成侵权。

本版形态下的实际要求分两种情况：

| 情况 | 义务 |
| --- | --- |
| 只学习它的做法（配置文件字段、写入顺序、回滚策略），代码自己写 | 没有法律义务。做法、思路、文件格式不受版权保护 |
| 直接复制粘贴了它的源码片段 | 该文件需保留 MIT 声明与版权行 |

建议的执行口径：配置层按自己的结构重写，不整段搬运；确实照搬了某个函数实现的地方，在文件头注明来源与 MIT。另外在关于页放一句「参考了开源项目 CC Switch 的实现思路」，这属于工程礼节，不是法律要求，但对开源生态和自身口碑都有好处。

同时继续遵守本仓库政策：不得移除或替换 new-api、QuantumNous 的品牌、归属与元数据。

## 3. 术语

| 术语 | 含义 |
| --- | --- |
| momoToken | 已上线服务端（new-api 定制部署，momotoken.win） |
| momo 登录器 | 本方案要做的桌面客户端，品牌沿用 momo·摸摸 |
| 官方 Agent | Claude Desktop（含 Code）、Claude Code CLI、Codex |
| 分组 | momoToken 的价格/上游分组，如 default、k12、gemini-anti |
| 接入 | 把 momoToken 的地址与 Key 写进某个官方 Agent 的配置，使其开始走我们的额度 |

## 4. 产品定位与非目标

登录器只做三件事：登录账号、管理额度、配置路由。

目标用户按优先级：完全不会用命令行的新手（首要）→ 想在多分组之间快速切换的老用户 → 已充值但配不起来的存量客户。

非目标：不做对话界面、不做文件编辑与工具调用、不做会话管理与 MCP 编排、不做本地代理进程、不做团队协作与企业策略、不做客户端内支付（跳网页）。

## 5. 支持范围

| 应用 | 平台 | 接入方式 | 首版 |
| --- | --- | --- | --- |
| Claude Desktop（含 Code） | macOS、Windows | 写第三方 Profile，指向 momoToken | 是（新手主入口） |
| Codex | macOS、Windows | 写 `auth.json` + `config.toml` | 是 |
| Claude Code CLI | macOS、Windows | 写配置，Anthropic 协议 | 是 |
| Gemini CLI | macOS、Windows | 写配置 | v1.1 |

跨平台用模型（Claude Desktop 用 GPT、Codex 用 Claude）由服务端转协议实现，客户端只负责选择与标注兼容等级。

## 6. 用户旅程

首次使用，目标 3 分钟内完成接入：

1. 安装 momo 登录器（macOS dmg / Windows exe）。
2. 在登录器里直接输入 momoToken 账号密码登录，含人机验证；开了 2FA 的走第二步验证。全程不跳浏览器。
3. 登录成功后台自动创建本机专属 Key，用户不感知。
4. 首页显示余额、今日消耗、当前分组。
5. 选「接入哪个应用」：Claude Desktop（默认高亮）、Codex、Claude Code CLI。未安装的给下载引导。
6. 选分组和模型：按分组展示，标注倍率与预估价格，跨平台组合标注兼容等级。
7. 点「接入」：备份原配置 → 写入新配置 → 提示重启目标应用。
8. 首页状态变「已就绪」，显示当前生效的应用 + 分组 + 模型。

日常使用：首页换分组或模型 → 点「切换」→ 重写配置。额度常驻首页，余额不足直接跳网页充值。

还原：一键「恢复官方登录」，从备份还原。

## 7. 信息架构

| 页面 | 内容 |
| --- | --- |
| 首页 | 当前接入状态、余额与今日消耗、快速切换分组与模型 |
| 应用 | 已检测到的官方 Agent、安装状态、接入/停用、恢复官方登录 |
| 模型 | 按分组展示可用模型、倍率与预估价、兼容等级 |
| 用量 | 近 7/30 天消耗，按模型与分组拆分，跳网页详单 |
| 设置 | 账号与设备、开机自启、语言、日志与诊断包、检查更新 |
| 帮助 | 新手指引、故障自查、第一句提示词示例、跳文档与充值 |

## 8. 技术架构

```
momo 登录器（Tauri 2 + React，全部自有代码）
  ├─ 账号模块    原生登录（含人机验证与 2FA）、Key 自动签发、凭证入 Keychain / CredMgr
  ├─ 额度与选型  余额、今日消耗、分组与模型、倍率与预估价、兼容等级
  ├─ 配置引擎    读写三类 Agent 配置、原子写、备份与回滚、环境探测、重启引导
  └─ 外围        托盘、开机自启、自检、诊断包、自动更新
          │  HTTPS，无本地代理进程
          ▼
   momotoken.win（new-api）：账号、余额、分组、模型、倍率、用量、协议转换
```

配置引擎要覆盖的写入目标（做法参考 CC Switch 已验证的字段与时序，代码自写）：

| 目标 | 落盘位置 | 关键点 |
| --- | --- | --- |
| Codex | `~/.codex/auth.json` + `~/.codex/config.toml` | 两文件必须同时生效，第二个写失败要回滚第一个；设置 `model_provider` 与 `model_providers.<id>.base_url`、`wire_api` |
| Claude Desktop | macOS `~/Library/Application Support/Claude/`、Windows `%LOCALAPPDATA%\Claude\`，第三方槽位用独立目录 | 写网关地址、Key 与模型映射；Claude Desktop 只认 Sonnet / Opus / Haiku 三个角色，需把角色映射到真实模型 |
| Claude Code CLI | 用户级配置 | Anthropic 协议直连 momoToken |

三条通用要求：写前备份、原子写（先写临时文件再重命名）、失败自动还原；切换后明确提示需要完全退出并重启目标应用，不静默失败。

## 9. 兼容等级

| 等级 | 判定 | UI |
| --- | --- | --- |
| 原生 | 模型与应用同协议同厂商 | 绿色「完整体验」 |
| 良好 | 跨厂商，但工具调用、流式、长上下文均可用 | 蓝色，列出细微差异 |
| 有限 | 缺部分能力（无并行工具调用、无缓存计费、无思考模式等） | 黄色，逐条列出 |
| 不建议 | 已知导致主要功能失效 | 灰色，需二次确认 |

矩阵由服务端下发，客户端只渲染，调整无需发版。等级判定的事实依据来自服务端 12 条转换路由的实测结果，不靠推测。

## 10. 安全设计

1. 密码只用于一次登录请求，不落盘、不写日志；登录后改用访问令牌。
2. 令牌与 API Key 存 macOS Keychain / Windows Credential Manager，UI 永不展示 Key。
3. 每台设备签发独立 Key（名称含设备标识），网页端可查看与撤销，丢机不影响其他设备。
4. 保留服务端人机验证与失败限流，不为了客户端体验削弱风控。
5. 写配置前原子备份，失败自动回滚。
6. 诊断日志默认脱敏，不含 Key、密码与请求正文。

## 11. 服务端改动（momotoken-new-api）

已有、可直接用：

| 接口 | 用途 |
| --- | --- |
| `POST /api/user/login` | 账号密码登录（客户端内渲染人机验证） |
| `GET /api/user/self/token` | 换访问令牌 |
| `GET /api/user/self` | 余额、分组、用户信息 |
| `GET /api/user/self/models` | 按可用分组返回模型列表 |
| `POST /api/token`、`GET /api/token/:id/key` | 创建并取回专属 Key |
| `GET /api/log/self`、`GET /api/data/self` | 用量与消耗统计 |
| `GET /api/pricing`、`GET /api/models` | 倍率与定价 |
| `POST /v1/messages`、`/v1/chat/completions` 等 | 协议入口，跨协议转换在服务端完成 |

需要新增（比 v2 少了一整套设备授权流程）：

| 改动 | 用途 |
| --- | --- |
| 登录器引导包接口 | 一次返回分组、模型、倍率、兼容矩阵、公告、客户端最低版本 |
| 设备列表 / 撤销（网页端 + 接口） | 管理各设备签发的 Key |
| 客户端登录风控加固 | 失败次数锁定、按设备限流；若 Turnstile 在 WebView 内不可用，则提供豁免 Turnstile 的客户端专用登录接口 |

## 12. 错误与提示（人话版）

| 场景 | 提示要点 |
| --- | --- |
| Claude Desktop 未重启 | 「需要完全退出 Claude 再打开」，附一键退出 |
| 人机验证失败 | 提示重试，连续失败给网页登录兜底链接 |
| 2FA 验证码错误 | 明确剩余尝试次数 |
| 余额不足 | 显示剩余额度并跳充值页 |
| 分组不可用 | 提示切换分组，给出可用替代 |
| 配置写入失败 | 已自动还原，附诊断包导出 |
| 客户端版本过低 | 引导包返回最低版本，提示升级 |

## 13. 成功指标与验收

- 新手从安装到接入完成 ≤ 3 分钟，零手填配置项，零跳浏览器。
- 首次接入成功率 ≥ 90%，失败均有明确指引。
- 切换分组或模型 ≤ 3 次点击。
- 一键恢复官方登录成功率 100%。
- 首版验收：macOS 与 Windows 各完成 Claude Desktop、Codex、Claude Code CLI 的接入、切换、还原全流程。

## 14. Epic 与子 Issue

相比 v2：删除「深链下发」与「fork 上游」相关工作，删除本地网关，删除设备授权流程；新增原生登录与配置引擎。

| Epic | 子 Issue | 依赖 | 验收 |
| --- | --- | --- | --- |
| E1 服务端 | 引导包接口、设备列表与撤销、登录风控加固 | 无 | 联调通过，风控生效 |
| E2 客户端骨架 | 仓库初始化、Tauri 2 + React、品牌与设计、双平台打包与 CI（macOS 签名公证，Windows 未签名产物 + 输出 SHA256） | 无 | 双平台产物可安装启动，macOS 公证通过 |
| E3 原生登录 | 登录页与人机验证、2FA 二次验证、Key 自动签发、凭证存储、登出与切换账号 | E1、E2 | 登录到拿到 Key 全链路通过，Key 不可见 |
| E4 额度与选型 | 余额与今日消耗、分组与模型列表、倍率与预估价、兼容等级标签 | E1、E3 | 数据与网页端一致，3 次点击内切换 |
| E5 配置引擎 | 原子写与备份回滚框架、环境探测、重启引导、恢复官方登录 | E3 | 写入失败必回滚，还原成功率 100% |
| E6 三个接入目标 | Codex、Claude Desktop、Claude Code CLI 各自的配置写入与实机验证 | E5 | 接入后无需手动改配置即可运行 |
| E7 跨协议验证 | `/v1/messages` 配非 Anthropic 上游的流式与工具调用实测，产出兼容矩阵初值 | E1 | 每个组合有实测结论，矩阵可下发 |
| E8 稳定性与体验 | 自检、错误文案、诊断包、自动更新、开机自启、托盘 | E6 | 常见故障可自查，诊断包脱敏 |
| E9 文档与发布 | 新手教程、官网下载页、版本说明、Windows SmartScreen 图文引导与哈希公示 | E6、E7 | momotoken.win 可下载并按文档跑通；Windows 用户能照图文自行完成安装 |

流程沿用 `.github/ISSUE_WORKFLOW.md`：`ops:managed` 纳管，状态标签 `status:triage / planned / in-progress / verify / blocked / done`，评论命令 `/triage` `/plan` `/start` `/verify` `/block <原因>` `/done`，分支 `codex/issue-<编号>-<slug>`。客户端仓库建库时复制同一套模板与 `momo-issue-control.yml`，当前 Codex 对话继续作为主控台。

## 15. 风险与应对

| 风险 | 应对 |
| --- | --- |
| Turnstile 组件在 WebView 内表现不佳 | 备选客户端专用登录接口 + 强化限流；最坏情况保留一次性网页登录兜底 |
| 客户端内登录被撞库利用 | 失败锁定、设备限流、异常登录邮件提醒 |
| 服务端跨协议转换质量不足 | E7 先实测再对外承诺；不达标的组合直接标「不建议」或隐藏 |
| Claude Desktop 更新导致配置字段变化 | 版本探测 + 服务端下发适配参数 + 失败即还原 |
| 官方限制第三方端点 | 保留 CLI 路线为主备，及时公告 |
| 配置写入损坏用户原有环境 | 原子写 + 强制备份 + 一键还原，作为 E5 的验收硬指标 |
| 复制了 CC Switch 源码片段 | 该文件保留 MIT 与版权行；优先自行重写（见第 2 节） |
| Windows 未签名导致安装流失 | 已知并接受的首版成本：下载页图文引导 + 哈希公示 + 文档站杀软误报处理（见 16.2）；观察流失情况再决定是否提前买证书 |

## 16. 签名与公证

两个平台的处理方式不同，因为未签名的后果不对称：macOS 未签名会被 Gatekeeper 直接拒绝启动，用户无法自行绕过，必须签名 + 公证；Windows 未签名只是弹 SmartScreen 警告，用户点两下可继续，属于可引导。因此首版策略是 macOS 正式签名公证，Windows 暂不办证书、用图文引导覆盖。

### 16.1 macOS：用自有 Apple 开发者账号（已定）

| 项 | 内容 |
| --- | --- |
| 账号 | 个人 Apple Developer Program，每年 99 美元 |
| 证书类型 | Developer ID Application（用于 App Store 外分发），**不是** Mac App Store 证书 |
| 必须开启 | Hardened Runtime，否则公证不通过 |
| 公证工具 | `notarytool`（`altool` 已废弃），用 App Store Connect API Key 或 app-specific password |
| 装订 | 公证通过后对 dmg 执行 `stapler staple`，让离线环境也能验证 |
| CI 凭证 | 证书导出 p12 后 base64 存入 GitHub Secrets，配合 Team ID 与 API Key |

Tauri 原生支持这条链路，配置签名身份与公证凭证后 `tauri build` 自动完成签名、公证、装订。首次公证通常几分钟内返回。

### 16.2 Windows：首版不办证书，用图文引导过 SmartScreen（已定）

决策：Windows 首版不购买代码签名证书，发未签名安装包，配一套图文引导让用户自己通过 SmartScreen 提示。

这个取舍成立的原因：Windows 的未签名后果比 macOS 轻。macOS 未签名会被 Gatekeeper 直接拒绝启动，用户无法自行绕过，所以必须签；Windows 只是弹一个 SmartScreen 警告，用户点两下就能继续，是可引导的。而当前 Windows 证书的成本与周期都不划算 —— 2023 年 6 月起行业规则要求私钥必须存放在专用硬件中，廉价软件证书已不再签发，剩下的选项要么有地区限制（Azure Trusted Signing 的个人身份仅限美国、加拿大），要么需要公司主体，要么办理周期一到两周。首版没必要把发布时间押在这上面。

需要接受的代价，先讲清楚不做美化：

- 首次运行必现 SmartScreen 蓝色弹窗，标题是「Windows 已保护你的电脑」，默认只有一个「不运行」按钮，「仍要运行」藏在「更多信息」里。
- 部分浏览器下载时会额外提示「不常下载，可能有危害」，需要用户手动选择保留。
- 少数第三方杀软可能拦截未签名的新程序。
- 这一步一定会流失一部分最谨慎的用户，属于已知成本。

因此图文引导不是附属说明，而是 Windows 端的正式交付物，要求：

| 位置 | 内容要求 |
| --- | --- |
| 官网下载页（momotoken.win） | Windows 下载按钮旁直接展示三步截图：更多信息 → 仍要运行 → 安装完成。截图用真实系统截屏，标注红框，不用文字描述代替 |
| 浏览器下载提示 | 针对 Edge / Chrome 的「保留」操作各给一张截图 |
| 安全说明 | 公示安装包 SHA256，并说明为什么会有这个提示（未购买代码签名证书，非病毒），语气坦诚不遮掩 |
| 文档站 | 同一套图文进 Windows 安装章节，并列出杀软误报时的处理方式（添加信任、或从官网重新下载校验哈希） |
| 客户端内 | 不需要处理，提示发生在安装前 |

后续升级路径（不在首版范围）：等下载量起来、或注册公司主体后再买证书。届时可选 Certum 的个人开源代码签名证书（69 欧元起，个人可办）或公司主体的 OV/EV 证书（EV 有即时 SmartScreen 信誉）。注意即使买了普通 OV 证书，初期仍需累积下载信誉才能完全消除提示，只有 EV 是立即生效的。

### 16.3 待决策与待验证

待决策：

1. 客户端仓库名与是否开源。
2. 何时以及以何种主体购买 Windows 代码签名证书（首版已定不买，见 16.2）。

待验证：

1. Turnstile 在 Tauri WebView 内的实际可用性（决定登录方案主备）。
2. `POST /v1/messages` 配 OpenAI / Gemini 上游时的流式与工具调用兼容度。
3. Codex 与 Claude Code CLI 在 Windows 的配置路径与写入行为实机确认。
4. Claude Desktop 各版本第三方槽位字段稳定性按版本记录。
5. Claude Desktop 三角色到真实模型的映射在跨厂商模型下的表现。
6. Windows 未签名安装包在主流环境的实际拦截强度：Edge / Chrome 下载提示、SmartScreen 弹窗文案、常见杀软（Defender、360、火绒）是否误报，用于确定图文引导要覆盖哪些场景。
