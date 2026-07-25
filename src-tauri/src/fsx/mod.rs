use std::fs;
use std::io::Write;
use std::path::Path;

/// 原子写：先写 .tmp，成功后 rename。
/// E2-4 中扩展为快照 + 回滚，此处为最小可用实现。
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.flush()?;
    }
    fs::rename(&tmp, path)
}
