// Piko 自身的安装说明：macOS / Windows 双平台图文引导
// 两个平台都未购买签名证书，首次安装会被系统拦一次，这里说明如何放行

import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { BRAND } from "../lib/brand";

const CARD = "rounded-2xl border border-black/5 bg-white p-4 shadow-sm dark:border-white/10 dark:bg-white/5";
const SUBTLE = "text-xs text-gray-500 dark:text-gray-400";
const CODE = "rounded bg-black/[0.04] px-1 text-gray-700 dark:bg-white/10 dark:text-gray-300";
const STRONG = "mx-1 font-medium text-gray-800 dark:text-gray-200";

type Platform = "macos" | "windows";

/** 首次进来默认停在当前系统那一页 */
function detectPlatform(): Platform {
  return navigator.userAgent.includes("Windows") ? "windows" : "macos";
}

export default function InstallGuide() {
  const navigate = useNavigate();
  const [platform, setPlatform] = useState<Platform>(detectPlatform);

  return (
    <div className="flex h-screen flex-col bg-transparent">
      <header className="flex items-center gap-3 border-b border-black/5 px-5 py-3 dark:border-white/10">
        <button
          onClick={() => navigate("/home")}
          aria-label="返回首页"
          className="text-gray-500 transition hover:text-gray-900 dark:text-gray-400 dark:hover:text-white"
        >
          ←
        </button>
        <h1 className="text-sm font-semibold text-gray-900 dark:text-white">
          {BRAND.name} 安装说明
        </h1>
      </header>

      <main className="flex-1 overflow-y-auto px-5 py-4">
        <div className="mx-auto max-w-3xl space-y-3">
          <div className={CARD}>
            <p className="text-sm font-semibold text-gray-900 dark:text-gray-100">
              为什么系统会拦一下？
            </p>
            <p className={`mt-1 ${SUBTLE}`}>
              {BRAND.fullName} 目前没有购买代码签名证书，macOS 的 Gatekeeper 和 Windows 的
              SmartScreen 都会对未知发布者的安装包弹一次提示。这是系统的标准行为，不代表程序有安全风险。
              所有版本都在 GitHub Actions 公开构建，可用 SHA256 校验和自行核验下载文件。
            </p>
          </div>

          <div className="flex gap-1 border-b border-black/5 dark:border-white/10">
            {([["macos", "🍎 macOS"], ["windows", "🪟 Windows"]] as const).map(([id, label]) => (
              <button
                key={id}
                onClick={() => setPlatform(id)}
                className={`-mb-px border-b-2 px-3 py-2 text-xs transition ${
                  platform === id
                    ? "border-gray-900 font-medium text-gray-900 dark:border-white dark:text-white"
                    : "border-transparent text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200"
                }`}
              >
                {label}
              </button>
            ))}
          </div>

          <div className={CARD}>
            {platform === "macos" ? (
              <ol className="space-y-5">
                <Step num={1} title="下载并打开 dmg">
                  <p className={SUBTLE}>
                    下载 <code className={CODE}>Piko_x.y.z_aarch64.dmg</code>（Apple 芯片）或{" "}
                    <code className={CODE}>x64.dmg</code>（Intel 芯片），双击打开后把{" "}
                    <code className={CODE}>{BRAND.name}.app</code> 拖进「应用程序」文件夹。
                  </p>
                </Step>

                <Step num={2} title="首次启动会提示无法验证开发者">
                  <Note title="📘 提示">
                    弹窗文字通常是「无法打开“{BRAND.name}”，因为 Apple 无法检查其是否包含恶意软件」，
                    此时只有「移到废纸篓」和「好」两个按钮，直接双击是打不开的。
                  </Note>
                </Step>

                <Step num={3} title="用右键菜单打开">
                  <p className={SUBTLE}>
                    在「应用程序」里
                    <span className={STRONG}>按住 Control 点击</span>
                    （或右键）{BRAND.name} 图标，选择
                    <span className={STRONG}>打开</span>，
                    在新弹窗里再点一次「打开」即可。这一步只需做一次，之后正常双击启动。
                  </p>
                </Step>

                <Step num={4} title="如果右键也没有打开选项">
                  <p className={SUBTLE}>
                    打开
                    <span className={STRONG}>系统设置 › 隐私与安全性</span>
                    ，向下滚动到「安全性」区域，会看到「已阻止使用“{BRAND.name}”」，点右侧的
                    <span className={STRONG}>仍要打开</span>
                    并输入密码确认。
                  </p>
                </Step>
              </ol>
            ) : (
              <ol className="space-y-5">
                <Step num={1} title="下载安装包">
                  <p className={SUBTLE}>
                    从 GitHub Releases 下载最新的 <code className={CODE}>*.msi</code> 或{" "}
                    <code className={CODE}>*_setup.exe</code>。下载前可对照页面公示的 SHA256
                    校验和核验文件完整性。
                  </p>
                </Step>

                <Step num={2} title="运行安装包，出现 SmartScreen 弹窗">
                  <Note title="📘 提示">
                    弹窗标题通常为「Windows 已保护你的电脑」，这是 Windows
                    对未知发布者安装包的标准提示，不代表程序存在安全风险。
                  </Note>
                </Step>

                <Step num={3} title="点击更多信息">
                  <p className={SUBTLE}>
                    在弹窗左下角点击蓝色的
                    <span className="mx-1 font-medium text-blue-600 dark:text-blue-400">更多信息</span>
                    链接，弹窗会展开显示发布者信息和额外按钮。
                  </p>
                </Step>

                <Step num={4} title="点击仍要运行完成安装">
                  <p className={SUBTLE}>
                    点击展开后出现的
                    <span className={STRONG}>仍要运行</span>
                    按钮，安装程序将正常启动，之后按提示完成安装即可。
                  </p>
                </Step>
              </ol>
            )}
          </div>

          <div className={CARD}>
            <p className="text-sm font-semibold text-gray-900 dark:text-gray-100">
              还没装 ChatGPT 桌面端或 Claude 桌面端？
            </p>
            <p className={`mt-1 ${SUBTLE}`}>
              {BRAND.name} 负责把账号和模型配置写进这些应用，本身不替代它们。请先从官网装好
              ChatGPT 桌面端或 Claude 桌面端，再回到首页选择应用并一键接入，
              {BRAND.name} 会自动检测到已安装的应用。
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}

function Note({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mt-2 rounded-xl border border-blue-500/30 bg-blue-500/5 p-4">
      <p className="text-xs font-semibold text-blue-700 dark:text-blue-300">{title}</p>
      <p className="mt-1 text-xs text-blue-700 dark:text-blue-200">{children}</p>
    </div>
  );
}

function Step({ num, title, children }: { num: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-4">
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-indigo-500/10 text-xs font-bold text-indigo-600 dark:text-indigo-400">
        {num}
      </div>
      <div className="min-w-0 pt-0.5">
        <p className="text-sm font-medium text-gray-900 dark:text-white">{title}</p>
        <div className="mt-1">{children}</div>
      </div>
    </li>
  );
}
