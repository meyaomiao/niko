use serde::{Deserialize, Serialize};

pub trait Target: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn is_installed(&self) -> bool;
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
