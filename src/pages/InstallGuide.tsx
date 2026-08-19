// Niko 自身的安装说明：macOS / Windows 双平台图文引导
// macOS 发布包已签名并公证，正常安装不会被拦；Windows 安装包尚未签名，需要说明如何越过 SmartScreen

import { useState } from "react";
import { useNavigate } from "react-router-dom";
import Logo from "../components/Logo";
import { BRAND } from "../lib/brand";
import { ArrowLeftIcon } from "../components/Icons";

const CARD = "nk-card";
const SUBTLE = "nk-muted";
const CODE = "rounded-md bg-[var(--nk-surface-muted)] px-1.5 py-0.5 text-gray-700 dark:text-gray-300";
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
    <div className="nk-shell">
      <header className="nk-header">
        <button
          onClick={() => navigate("/home")}
          aria-label="返回首页"
          className="nk-btn-ghost px-2.5"
        >
          <ArrowLeftIcon />
        </button>
        <Logo size={24} />
        <h1 className="nk-title">安装说明</h1>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-3xl space-y-3">
          <div className={CARD}>
            <p className="text-sm font-semibold text-gray-900 dark:text-gray-100">
              两个平台的安装体验不一样
            </p>
            <p className={`mt-1 ${SUBTLE}`}>
              macOS 安装包已使用 Developer ID 证书签名并通过 Apple 公证，正常下载安装不会被
              Gatekeeper 拦下。Windows 安装包目前还没有代码签名，首次运行会弹一次 SmartScreen
              提示，这是系统对未知发布者的标准行为，不代表程序有安全风险。
              每个版本都随 Release 附带 SHA256 校验和文件，可以自行核验下载到的安装包。
            </p>
          </div>

          <div className="flex gap-1 border-b [border-color:var(--nk-line)]">
            {([["macos", "macOS"], ["windows", "Windows"]] as const).map(([id, label]) => (
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
                    下载 <code className={CODE}>{`Niko_${BRAND.version}_universal.dmg`}</code>
                    ，同一个安装包同时支持 Apple 芯片和 Intel 芯片，不需要区分机型。双击打开后把{" "}
                    <code className={CODE}>{BRAND.name}.app</code> 拖进「应用程序」文件夹。
                  </p>
                </Step>

                <Step num={2} title="直接双击启动">
                  <p className={SUBTLE}>
                    安装包已签名并公证，第一次打开时系统只会确认一次「从互联网下载」，点
                    <span className={STRONG}>打开</span>
                    即可，不需要右键打开或到系统设置里放行。
                  </p>
                </Step>

                <Step num={3} title="想核验安装包（可选）">
                  <p className={SUBTLE}>
                    从同一个 Release 下载 <code className={CODE}>SHA256SUMS.txt</code>
                    ，和 dmg 放在同一个目录里，在「终端」执行{" "}
                    <code className={CODE}>shasum -a 256 -c SHA256SUMS.txt</code>
                    ，看到 dmg 那一行显示 OK 即说明文件完整。
                  </p>
                </Step>

                <Step num={4} title="如果仍然提示无法验证开发者">
                  <Note title="提示">
                    这通常说明下载没有完成或文件被改动过，建议先从 GitHub Releases
                    重新下载一次。确认来源无误又需要放行时，打开
                    <span className={STRONG}>系统设置 › 隐私与安全性</span>
                    ，在「安全性」区域点「已阻止使用“{BRAND.name}”」右侧的
                    <span className={STRONG}>仍要打开</span>
                    并输入密码确认。
                  </Note>
                </Step>
              </ol>
            ) : (
              <ol className="space-y-5">
                <Step num={1} title="下载安装包">
                  <p className={SUBTLE}>
                    从 GitHub Releases 下载{" "}
                    <code className={CODE}>{`Niko_${BRAND.version}_x64-setup.exe`}</code> 或{" "}
                    <code className={CODE}>{`Niko_${BRAND.version}_x64_en-US.msi`}</code>
                    ，两者装出来的是同一个应用，任选其一。需要核验时，同时下载{" "}
                    <code className={CODE}>SHA256SUMS_win.txt</code>，在 PowerShell 执行{" "}
                    <code className={CODE}>Get-FileHash 安装包名</code> 并与文件中的哈希比对。
                  </p>
                </Step>

                <Step num={2} title="运行安装包，出现 SmartScreen 弹窗">
                  <Note title="提示">
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
              本应用负责把账号和模型设置接入这些应用，本身不替代它们。请先从官网装好
              ChatGPT 桌面端或 Claude 桌面端，再回到首页选择应用并一键接入，
              本应用会自动检查应用是否已经安装。
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}

function Note({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="nk-alert-info mt-2 p-4">
      <p className="text-xs font-semibold">{title}</p>
      <p className="mt-1 text-xs">{children}</p>
    </div>
  );
}

function Step({ num, title, children }: { num: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-4">
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[var(--nk-info-soft)] text-xs font-bold text-[var(--nk-info)]">
        {num}
      </div>
      <div className="min-w-0 pt-0.5">
        <p className="text-sm font-medium text-gray-900 dark:text-white">{title}</p>
        <div className="mt-1">{children}</div>
      </div>
    </li>
  );
}
