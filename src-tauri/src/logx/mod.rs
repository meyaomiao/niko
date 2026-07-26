//! E7-3: 会话日志（脱敏）+ 轮转
//!
//! 日志写到 `~/.momo-launcher/logs/session.log`（Windows 为 %APPDATA%）。
//! 所有写入前都会做一次密钥脱敏，导出时再做一次，保证文件中不含完整 Key。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn base_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:\\Users\\default\\AppData\\Roaming"));
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    base.join(".momo-launcher")
}

pub fn session_log_path() -> PathBuf {
    base_dir().join("logs").join("session.log")
}

/// 只保留首尾各 4 个字符，其余用 * 代替。
pub fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}***{tail}")
}

/// 扫描一行文本，把疑似密钥的 token 脱敏。
/// 判定条件：以 sk- / Bearer 后跟的长 token，或长度 >= 24 且只含 key 常见字符。
pub fn redact_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut token = String::new();

    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            if looks_like_secret(token) {
                out.push_str(&mask_secret(token));
            } else {
                out.push_str(token);
            }
            token.clear();
        }
    };

    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            token.push(ch);
        } else {
            flush(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn looks_like_secret(token: &str) -> bool {
    if token.starts_with("sk-") && token.len() > 8 {
        return true;
    }
    if token.len() < 24 {
        return false;
    }
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
    has_digit && has_alpha
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单的 UTC 时间格式化，避免额外依赖
    let days = secs / 86_400;
    let rem = secs % 86_400;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_date(days),
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_date(days_since_epoch: u64) -> String {
    // 1970-01-01 起的天数换算为 y-m-d（Howard Hinnant 的 civil_from_days 算法）
    let z = days_since_epoch as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// 追加一行日志。写入失败时静默忽略，日志不应影响主流程。
pub fn append(scope: &str, message: &str) {
    let path = session_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = fs::rename(&path, path.with_extension("log.1"));
        }
    }

    let line = format!("[{}] [{}] {}\n", timestamp(), scope, redact_line(message));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 读取日志尾部内容（最多 max_bytes 字节）。
pub fn read_tail(max_bytes: usize) -> String {
    let path = session_log_path();
    let content = fs::read_to_string(&path).unwrap_or_default();
    if content.len() <= max_bytes {
        return content;
    }
    let start = content.len() - max_bytes;
    let cut = content[start..]
        .find('\n')
        .map(|i| start + i + 1)
        .unwrap_or(start);
    content[cut..].to_owned()
}
