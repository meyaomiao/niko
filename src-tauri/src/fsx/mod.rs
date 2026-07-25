//! fsx — 原子写与快照回滚
//!
//! 写入流程：
//!   1. 读取原始文件到 `.snap` 快照
//!   2. 写入新内容到 `.tmp` 临时文件
//!   3. `rename` `.tmp` → 目标（原子替换）
//!   4. 删除 `.snap`
//!
//! 如果步骤 2/3 失败，调用 `rollback` 可将 `.snap` 恢复为目标文件。
//! 如果目标文件原本不存在，`.snap` 不会被创建，回滚时直接删除目标文件。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 表示一次写入操作前保存的快照，可用于回滚。
pub struct Snapshot {
    /// 目标文件路径
    target: PathBuf,
    /// 快照文件路径（`.snap`），`None` 表示目标原本不存在
    snap: Option<PathBuf>,
}

impl Snapshot {
    /// 将快照文件（或"不存在"状态）恢复到目标路径。
    pub fn rollback(self) -> std::io::Result<()> {
        match self.snap {
            Some(snap) => {
                fs::rename(&snap, &self.target)?;
            }
            None => {
                // 目标原本不存在，删除写入的文件（忽略不存在错误）
                match fs::remove_file(&self.target) {
                    Ok(()) | Err(_) => {}
                }
            }
        }
        Ok(())
    }

    /// 确认写入成功后，清理快照文件。
    pub fn commit(self) {
        if let Some(snap) = self.snap {
            let _ = fs::remove_file(snap);
        }
    }
}

/// 原子写入 `path`，写入前先保存快照。
///
/// 返回 `Snapshot`，调用者决定 `.commit()` 或 `.rollback()`。
pub fn write_with_snapshot(path: &Path, content: &[u8]) -> std::io::Result<Snapshot> {
    let snap_path = path.with_extension("snap");
    let tmp_path = path.with_extension("tmp");

    // 1. 保存快照
    let snap = if path.exists() {
        fs::copy(path, &snap_path)?;
        Some(snap_path)
    } else {
        None
    };

    // 2. 写入 .tmp
    let write_result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(content)?;
        f.flush()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        // 写入失败，清理 .tmp，快照保留供调用者决定
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    // 3. 原子替换
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(Snapshot { target: path.to_path_buf(), snap })
}

/// 简便函数：写入后立即 commit（无需回滚能力）。
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    write_with_snapshot(path, content).map(|s| s.commit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fsx_test_{name}"))
    }

    #[test]
    fn write_atomic_creates_file() {
        let p = tmp_file("create");
        let _ = fs::remove_file(&p);
        write_atomic(&p, b"hello").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"hello");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn write_atomic_overwrites_existing() {
        let p = tmp_file("overwrite");
        fs::write(&p, b"old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn snapshot_rollback_restores_original() {
        let p = tmp_file("rollback");
        fs::write(&p, b"original").unwrap();
        let snap = write_with_snapshot(&p, b"modified").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"modified");
        snap.rollback().unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"original");
        // .snap 应已被清理
        assert!(!p.with_extension("snap").exists());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn snapshot_rollback_removes_new_file_when_original_absent() {
        let p = tmp_file("rollback_new");
        let _ = fs::remove_file(&p);
        let snap = write_with_snapshot(&p, b"new file").unwrap();
        assert!(p.exists());
        snap.rollback().unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn snapshot_commit_cleans_snap_file() {
        let p = tmp_file("commit");
        fs::write(&p, b"v1").unwrap();
        let snap = write_with_snapshot(&p, b"v2").unwrap();
        snap.commit();
        assert_eq!(fs::read(&p).unwrap(), b"v2");
        assert!(!p.with_extension("snap").exists());
        let _ = fs::remove_file(&p);
    }
}
