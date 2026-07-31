use serde::Serialize;

pub const SAFE_ERROR_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SafeCommandError {
    pub version: u16,
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'static str>,
}

impl SafeCommandError {
    pub const fn new(
        code: &'static str,
        message: &'static str,
        retryable: bool,
        action: Option<&'static str>,
    ) -> Self {
        Self {
            version: SAFE_ERROR_VERSION,
            code,
            message,
            retryable,
            action,
        }
    }

    pub const fn invalid_request() -> Self {
        Self::new("invalid_request", "请求无效，请重新检查。", false, None)
    }

    pub const fn read_failed() -> Self {
        Self::new("read_failed", "本地内容暂时无法读取。", true, Some("retry"))
    }

    pub const fn busy() -> Self {
        Self::new(
            "busy",
            "另一个操作正在进行，请稍后再试。",
            true,
            Some("retry"),
        )
    }

    pub const fn change_failed(retryable: bool) -> Self {
        Self::new(
            "change_failed",
            "操作未完成，原有内容保持可用。",
            retryable,
            if retryable { Some("retry") } else { None },
        )
    }

    pub const fn open_failed() -> Self {
        Self::new(
            "open_failed",
            "未能打开应用，请手动打开。",
            true,
            Some("retry"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_error_is_versioned_and_redacted() {
        let errors = [
            SafeCommandError::invalid_request(),
            SafeCommandError::read_failed(),
            SafeCommandError::busy(),
            SafeCommandError::change_failed(false),
            SafeCommandError::change_failed(true),
            SafeCommandError::open_failed(),
        ];
        for error in errors {
            let json = serde_json::to_string(&error).unwrap();
            assert_eq!(error.version, SAFE_ERROR_VERSION);
            for forbidden in [
                "/Users/",
                "~/.codex/config.toml",
                "auth.json",
                "journal",
                "WAL",
                "SQLite",
                "custom",
                "lock",
                "API key",
                "token",
            ] {
                assert!(
                    !json.to_lowercase().contains(&forbidden.to_lowercase()),
                    "leaked {forbidden}: {json}"
                );
            }
        }
    }
}
