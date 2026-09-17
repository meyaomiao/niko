//! 面向用户手改过的 YAML：尽量按行保守改一个 mapping 块，保留旁边的注释。
//! 解析失败时拒绝写入。

use serde_yaml::Value;

fn is_blank_or_full_comment(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn content_indent(line: &str) -> Option<usize> {
    if is_blank_or_full_comment(line) {
        return None;
    }
    Some(line.chars().take_while(|ch| *ch == ' ').count())
}

fn key_line_prefix(indent: usize, key: &str) -> String {
    format!("{}{key}:", " ".repeat(indent))
}

fn line_is_key_at(line: &str, indent: usize, key: &str) -> bool {
    let prefix = key_line_prefix(indent, key);
    if !line.starts_with(&prefix) {
        return false;
    }
    line[prefix.len()..].starts_with([' ', '\t']) || line.trim_end() == prefix.trim_end()
}

fn block_end(lines: &[String], start: usize, indent: usize) -> usize {
    let mut end = start + 1;
    while end < lines.len() {
        let line = &lines[end];
        if is_blank_or_full_comment(line) {
            end += 1;
            continue;
        }
        if content_indent(line).is_some_and(|found| found <= indent) {
            break;
        }
        end += 1;
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    end
}

fn find_key(lines: &[String], from: usize, to: usize, indent: usize, key: &str) -> Option<usize> {
    let mut index = from;
    while index < to {
        if line_is_key_at(&lines[index], indent, key) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn indent_block(body: &str, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    body.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect()
}

fn ensure_parent_mapping(
    lines: &mut Vec<String>,
    path: &[&str],
) -> Result<(usize, usize, usize), String> {
    if path.is_empty() {
        return Ok((0, lines.len(), 0));
    }
    let mut range_start = 0usize;
    let mut range_end = lines.len();
    let mut indent = 0usize;
    for (depth, key) in path.iter().enumerate() {
        match find_key(lines, range_start, range_end, indent, key) {
            Some(index) => {
                let end = block_end(lines, index, indent);
                range_start = index + 1;
                range_end = end;
                indent += 2;
            }
            None if depth == 0 => {
                if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("{key}:"));
                range_start = lines.len();
                range_end = lines.len();
                indent = 2;
            }
            None => {
                let insert_at = range_end;
                lines.insert(insert_at, format!("{}{key}:", " ".repeat(indent)));
                range_start = insert_at + 1;
                range_end = insert_at + 1;
                indent += 2;
            }
        }
    }
    Ok((range_start, range_end, indent))
}

/// 在 `path` 指向的 mapping 下写入/替换 `child_key` 整个块。
/// `child_body` 是相对 0 缩进的 YAML 片段（不含 child_key 自身）。
pub fn upsert_child_mapping(
    raw: &str,
    path: &[&str],
    child_key: &str,
    child_body: &str,
) -> Result<(String, bool), String> {
    let mut lines: Vec<String> = if raw.is_empty() {
        Vec::new()
    } else {
        raw.split('\n').map(str::to_owned).collect()
    };
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let (range_start, range_end, indent) = ensure_parent_mapping(&mut lines, path)?;
    let expected = indent_block(child_body.trim_end_matches('\n'), indent + 2);
    let mut replacement = vec![format!("{}{child_key}:", " ".repeat(indent))];
    replacement.extend(expected);

    if let Some(existing) = find_key(&lines, range_start, range_end, indent, child_key) {
        let end = block_end(&lines, existing, indent);
        let current = &lines[existing..end];
        if current == replacement {
            let mut out = lines.join("\n");
            if !raw.ends_with('\n') && raw.contains('\n') {
                return Ok((out, false));
            }
            if raw.ends_with('\n') && !out.ends_with('\n') {
                out.push('\n');
            }
            return Ok((out, false));
        }
        lines.splice(existing..end, replacement);
    } else {
        let mut insert_at = range_end;
        while insert_at < lines.len() && lines[insert_at].trim().is_empty() {
            insert_at += 1;
        }
        if insert_at > 0 && insert_at == lines.len() && !lines[insert_at - 1].trim().is_empty() {
            lines.insert(insert_at, String::new());
            insert_at += 1;
        }
        lines.splice(insert_at..insert_at, replacement);
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok((out, true))
}

pub fn remove_child_mapping(raw: &str, path: &[&str], child_key: &str) -> Result<(String, bool), String> {
    let mut lines: Vec<String> = raw.split('\n').map(str::to_owned).collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut range_start = 0usize;
    let mut range_end = lines.len();
    let mut indent = 0usize;
    for key in path {
        let Some(index) = find_key(&lines, range_start, range_end, indent, key) else {
            return Ok((raw.to_owned(), false));
        };
        let end = block_end(&lines, index, indent);
        range_start = index + 1;
        range_end = end;
        indent += 2;
    }
    let Some(existing) = find_key(&lines, range_start, range_end, indent, child_key) else {
        return Ok((raw.to_owned(), false));
    };
    let end = block_end(&lines, existing, indent);
    lines.drain(existing..end);
    while existing > 0 && existing < lines.len() && lines[existing].trim().is_empty() {
        lines.remove(existing);
    }
    let mut out = lines.join("\n");
    if raw.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok((out, true))
}

pub fn upsert_top_mapping(raw: &str, key: &str, body: &str) -> Result<(String, bool), String> {
    upsert_child_mapping(raw, &[], key, body)
}

pub fn quote_yaml_scalar(value: &str) -> String {
    if value.is_empty()
        || value.bytes().any(|byte| {
            matches!(
                byte,
                b':' | b'#' | b'\'' | b'"' | b'{' | b'}' | b'[' | b']' | b',' | b'&' | b'*' | b'!'
                    | b'|' | b'>' | b'%' | b'@' | b'`'
            ) || byte.is_ascii_whitespace()
        })
    {
        serde_yaml::to_string(&Value::String(value.to_owned()))
            .unwrap_or_else(|_| format!("\"{value}\""))
            .trim()
            .to_owned()
    } else {
        value.to_owned()
    }
}

/// 在 `path` 下写入/替换一个标量键。
pub fn upsert_child_scalar(
    raw: &str,
    path: &[&str],
    child_key: &str,
    value: &str,
) -> Result<(String, bool), String> {
    let mut lines: Vec<String> = if raw.is_empty() {
        Vec::new()
    } else {
        raw.split('\n').map(str::to_owned).collect()
    };
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let (range_start, range_end, indent) = ensure_parent_mapping(&mut lines, path)?;
    let rendered = format!(
        "{}{child_key}: {}",
        " ".repeat(indent),
        quote_yaml_scalar(value)
    );
    if let Some(existing) = find_key(&lines, range_start, range_end, indent, child_key) {
        if lines[existing] == rendered {
            let mut out = lines.join("\n");
            if raw.ends_with('\n') && !out.ends_with('\n') {
                out.push('\n');
            }
            return Ok((out, false));
        }
        let end = block_end(&lines, existing, indent);
        lines.splice(existing..end, [rendered]);
    } else {
        lines.insert(range_end, rendered);
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok((out, true))
}

pub fn remove_child_scalar(raw: &str, path: &[&str], child_key: &str) -> Result<(String, bool), String> {
    remove_child_mapping(raw, path, child_key)
}

pub fn yaml_mapping_or_err(raw: &str, label: &str) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Ok(Value::Mapping(serde_yaml::Mapping::new()));
    }
    let value: Value =
        serde_yaml::from_str(raw).map_err(|error| format!("{label} 解析失败，未做任何修改：{error}"))?;
    if !value.is_mapping() && !value.is_null() {
        return Err(format!("{label} 根节点不是 mapping，未做任何修改"));
    }
    Ok(value)
}

pub fn mapping_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_mapping()?.get(Value::String(key.to_owned()))
}

pub fn mapping_str(value: &Value, key: &str) -> Option<String> {
    mapping_get(value, key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_preserves_sibling_comments_and_is_idempotent() {
        let raw = "\
ui-onboarding:
  welcomeNoticeVersion: 1
llm-pi-ai:
  providers:
    # keep me
    grok-cli:
      apiKeyEnv: GROK_CLI_API_KEY
      baseURL: http://127.0.0.1:8765/v1
agent-default-model:
  provider: grok
  model: grok-4.6
";
        let body = "\
displayName: Niko / momotoken
apiKeyEnv: MOMOTOKEN_API_KEY
api: openai-completions
baseURL: https://momotoken.win/v1
models:
  - id: gpt-5.4
";
        let (once, changed) =
            upsert_child_mapping(raw, &["llm-pi-ai", "providers"], "momotoken", body).unwrap();
        assert!(changed);
        assert!(once.contains("# keep me"));
        assert!(once.contains("grok-cli:"));
        assert!(once.contains("momotoken:"));
        assert!(once.contains("displayName: Niko / momotoken"));
        let (twice, changed_again) =
            upsert_child_mapping(&once, &["llm-pi-ai", "providers"], "momotoken", body).unwrap();
        assert!(!changed_again);
        assert_eq!(once, twice);
    }

    #[test]
    fn remove_only_our_child() {
        let raw = "\
llm-pi-ai:
  providers:
    grok-cli:
      api: openai-completions
    momotoken:
      api: openai-completions
      baseURL: https://momotoken.win/v1
";
        let (out, changed) =
            remove_child_mapping(raw, &["llm-pi-ai", "providers"], "momotoken").unwrap();
        assert!(changed);
        assert!(out.contains("grok-cli:"));
        assert!(!out.contains("momotoken:"));
    }
}
