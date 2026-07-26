// E9-3: Windows SmartScreen 图文安装引导页
// 仅在 Windows 环境下通过下载页链接展示，其他平台不会路由进来

export default function InstallGuide() {
  return (
    <div className="flex min-h-screen flex-col bg-gray-950 px-6 py-10">
      <div className="mx-auto max-w-lg">
        <div className="mb-6">
          <span className="text-3xl">🪟</span>
          <h1 className="mt-3 text-xl font-bold text-white">Windows 安装说明</h1>
          <p className="mt-1 text-sm text-gray-400">
            momo·摸摸 Windows 版当前未购买代码签名证书，首次安装时会触发 SmartScreen 保护提示。
            按以下步骤操作即可正常安装，程序本身完全安全。
          </p>
        </div>

        <ol className="space-y-5">
          <Step num={1} title="下载安装包">
            <p className="text-xs text-gray-400">
              从 GitHub Releases 下载最新的 <code className="rounded bg-gray-800 px-1 text-gray-300">*.msi</code> 或{" "}
              <code className="rounded bg-gray-800 px-1 text-gray-300">*_setup.exe</code> 文件。
              下载前可对照页面公示的 SHA256 校验和核验文件完整性。
            </p>
          </Step>

          <Step num={2} title="运行安装包，出现 SmartScreen 弹窗">
            <div className="mt-2 rounded-xl border border-blue-800/40 bg-blue-950/20 p-4">
              <p className="text-xs font-semibold text-blue-300">📘 提示</p>
              <p className="mt-1 text-xs text-blue-200">
                弹窗标题通常为"Windows 已保护你的电脑"，这是 Windows 对未知发布者安装包的标准提示，
                不代表程序存在安全风险。
              </p>
            </div>
          </Step>

          <Step num={3} title='点击"更多信息"'>
            <p className="text-xs text-gray-400">
              在 SmartScreen 弹窗左下角，点击蓝色的
              <span className="mx-1 font-medium text-blue-400">更多信息</span>
              链接，弹窗会展开显示发布者信息和额外按钮。
            </p>
          </Step>

          <Step num={4} title='点击"仍要运行"完成安装'>
            <p className="text-xs text-gray-400">
              点击展开后出现的
              <span className="mx-1 font-medium text-gray-200">仍要运行</span>
              按钮，安装程序将正常启动。之后按提示完成安装即可。
            </p>
          </Step>
        </ol>

        <div className="mt-8 rounded-xl border border-gray-800 bg-gray-900 p-4 text-xs text-gray-500">
          <p className="font-medium text-gray-400">为什么没有签名？</p>
          <p className="mt-1">
            代码签名证书每年费用较高，目前版本优先保证功能可用性。
            所有发布版本均在 GitHub Actions 公开构建流程中编译，源代码完全开源，
            可通过 SHA256 校验和自行核验下载文件的完整性。
          </p>
        </div>
      </div>
    </div>
  );
}

function Step({ num, title, children }: { num: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-4">
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-indigo-600/20 text-xs font-bold text-indigo-400">
        {num}
      </div>
      <div className="pt-0.5">
        <p className="text-sm font-medium text-white">{title}</p>
        <div className="mt-1">{children}</div>
      </div>
    </li>
  );
}
