import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * 页面级错误边界：任何渲染期异常都在界面上显示出来，
 * 而不是留下一个白屏（之前出现过「页面内容又是空的」这种无法排查的情况）。
 */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // 控制台保留完整堆栈，方便开发时定位
    console.error("页面渲染失败:", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="nk-shell">
        <main className="nk-page">
          <div className="mx-auto max-w-xl space-y-3">
            <section className="nk-card">
              <h1 className="nk-title">页面加载失败</h1>
              <p className="nk-muted mt-1 text-xs">
                这个页面在渲染时出错了，错误信息如下（可直接反馈给开发者）：
              </p>
              <pre className="mt-2 max-h-52 overflow-auto rounded-lg bg-[var(--nk-surface-muted)] p-3 text-[11px] whitespace-pre-wrap text-[var(--nk-danger)]">
                {error.message || String(error)}
              </pre>
              <div className="mt-3 flex gap-2">
                <button
                  onClick={() => this.setState({ error: null })}
                  className="nk-btn-ghost px-3 py-1.5 text-xs"
                >
                  重试
                </button>
                <button
                  onClick={() => {
                    this.setState({ error: null });
                    window.location.hash = "";
                    window.location.reload();
                  }}
                  className="nk-btn-primary px-3 py-1.5 text-xs"
                >
                  重新加载应用
                </button>
              </div>
            </section>
          </div>
        </main>
      </div>
    );
  }
}
