use serde::{Deserialize, Serialize};

/// 每种可接入目标（Codex、Claude Desktop、Claude Code CLI 等）的统一 trait。
/// E5-1 中实现具体类型，此处仅定义接口占位。
pub trait Target: Send + Sync {
    /// 目标唯一 ID，如 "codex", "claude-desktop", "claude-code"
    fn id(&self) -> &'static str;
    /// 人类可读名称
    fn display_name(&self) -> &'static str;
    /// 检测目标是否已安装
    fn is_installed(&self) -> bool;
    /// 将给定配置写入目标，返回操作摘要
    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub base_url: String,
    pub api_key: String,
    pub model_group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplySummary {
    pub target_id: String,
    pub changed_keys: Vec<String>,
}
