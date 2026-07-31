//! E5-5: 快照列表与恢复
//!
//! 每次 apply 前由 targets/mod.rs 写一份 `.bak` 到
//! `~/.niko/backups/{target_id}/v2_{timestamp}_{nonce}_{filename}`
//! 本模块提供列出和恢复的 Tauri 命令。

use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Component;
use std::path::{Path, PathBuf};

use crate::commands::safe_error::SafeCommandError;

const SNAPSHOT_CREATE_ATTEMPTS: usize = 32;
const SNAPSHOT_NONCE_HEX_LEN: usize = 8;
const SNAPSHOT_FILENAME_MAX_LEN: usize = 255;

fn backup_dir_for(target_id: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:\\Users\\default\\AppData\\Roaming"));
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    base.join(".niko").join("backups").join(target_id)
}

/// 把 `src` 文件备份到对应目标的备份目录，文件名加上时间戳前缀。
pub fn save_backup(target_id: &str, src: &Path) -> std::io::Result<()> {
    let dir = backup_dir_for(target_id);
    save_backup_at(&dir, src).map(|_| ())
}

fn save_backup_at(dir: &Path, src: &Path) -> io::Result<Option<PathBuf>> {
    save_backup_at_time(dir, src, std::time::SystemTime::now())
}

fn save_backup_at_time(
    dir: &Path,
    src: &Path,
    time: std::time::SystemTime,
) -> io::Result<Option<PathBuf>> {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "snapshot time is before epoch"))?
        .as_secs();
    let candidates = random_snapshot_nonces()?;
    save_backup_at_with_candidates(dir, src, seconds, candidates, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotWriteStep {
    Copy,
    Permissions,
    FileSync,
    ParentSync,
}

struct OwnedSnapshot {
    path: PathBuf,
    file: Option<File>,
    keep: bool,
}

impl OwnedSnapshot {
    fn create(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        Ok(Self {
            path: path.to_owned(),
            file: Some(file),
            keep: false,
        })
    }

    fn file(&self) -> &File {
        self.file.as_ref().expect("owned snapshot is open")
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("owned snapshot is open")
    }

    fn keep(mut self) -> PathBuf {
        self.file.take();
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for OwnedSnapshot {
    fn drop(&mut self) {
        self.file.take();
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn random_snapshot_nonces() -> io::Result<Vec<u32>> {
    let mut candidates = Vec::with_capacity(SNAPSHOT_CREATE_ATTEMPTS);
    for _ in 0..SNAPSHOT_CREATE_ATTEMPTS {
        let mut random = [0u8; 4];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        candidates.push(u32::from_ne_bytes(random));
    }
    Ok(candidates)
}

fn save_backup_at_with_candidates(
    dir: &Path,
    src: &Path,
    seconds: u64,
    candidates: impl IntoIterator<Item = u32>,
    mut before: impl FnMut(SnapshotWriteStep) -> io::Result<()>,
) -> io::Result<Option<PathBuf>> {
    let metadata = match fs::symlink_metadata(src) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid backup source",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let fname = src
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid snapshot filename"))?;
    validate_snapshot_original_name(fname)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid snapshot filename"))?;
    fs::create_dir_all(dir)?;

    for candidate in candidates.into_iter().take(SNAPSHOT_CREATE_ATTEMPTS) {
        let destination_path = dir.join(format_snapshot_name(seconds, candidate, fname)?);
        let mut destination = match OwnedSnapshot::create(&destination_path) {
            Ok(destination) => destination,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        before(SnapshotWriteStep::Copy)?;
        let mut source = File::open(src)?;
        io::copy(&mut source, destination.file_mut())?;
        before(SnapshotWriteStep::Permissions)?;
        secure_snapshot_permissions(fname, destination.file(), metadata.permissions())?;
        before(SnapshotWriteStep::FileSync)?;
        destination.file().sync_all()?;
        before(SnapshotWriteStep::ParentSync)?;
        sync_snapshot_directory(dir)?;
        return Ok(Some(destination.keep()));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "snapshot names are occupied",
    ))
}

#[cfg(unix)]
fn secure_snapshot_permissions(
    filename: &str,
    destination: &File,
    source_permissions: fs::Permissions,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = if filename == "auth.json" {
        fs::Permissions::from_mode(0o600)
    } else {
        source_permissions
    };
    destination.set_permissions(permissions)
}

#[cfg(not(unix))]
fn secure_snapshot_permissions(
    _filename: &str,
    destination: &File,
    source_permissions: fs::Permissions,
) -> io::Result<()> {
    destination.set_permissions(source_permissions)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedSnapshotName<'a> {
    seconds: u64,
    original_name: &'a str,
}

fn format_snapshot_name(seconds: u64, nonce: u32, original_name: &str) -> io::Result<String> {
    validate_snapshot_original_name(original_name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid snapshot filename"))?;
    Ok(format!("v2_{seconds}_{nonce:08x}_{original_name}"))
}

fn parse_snapshot_name(filename: &str) -> Option<ParsedSnapshotName<'_>> {
    if !is_single_path_component(filename) {
        return None;
    }
    if let Some(versioned) = filename.strip_prefix("v2_") {
        let mut fields = versioned.splitn(3, '_');
        let seconds = parse_decimal_seconds(fields.next()?)?;
        let nonce = fields.next()?;
        let original_name = fields.next()?;
        if nonce.len() != SNAPSHOT_NONCE_HEX_LEN
            || !nonce
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || u32::from_str_radix(nonce, 16).is_err()
            || validate_snapshot_original_name(original_name).is_err()
        {
            return None;
        }
        return Some(ParsedSnapshotName {
            seconds,
            original_name,
        });
    }

    let (seconds, original_name) = filename.split_once('_')?;
    let seconds = parse_decimal_seconds(seconds)?;
    validate_snapshot_original_name(original_name).ok()?;
    Some(ParsedSnapshotName {
        seconds,
        original_name,
    })
}

fn parse_decimal_seconds(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

fn validate_snapshot_original_name(name: &str) -> Result<(), ()> {
    if name.is_empty() || name.len() > SNAPSHOT_FILENAME_MAX_LEN || !is_single_path_component(name)
    {
        return Err(());
    }
    Ok(())
}

fn is_single_path_component(value: &str) -> bool {
    Path::new(value).components().count() == 1
        && matches!(
            Path::new(value).components().next(),
            Some(Component::Normal(_))
        )
}

#[cfg(unix)]
fn sync_snapshot_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_snapshot_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct SnapshotEntry {
    pub target_id: String,
    pub filename: String,
    /// Unix timestamp（秒）
    pub timestamp: u64,
    /// 原始文件名（去掉时间戳前缀）
    pub original_name: String,
}

/// 列出某目标的所有备份（按时间倒序）
#[tauri::command]
pub async fn list_snapshots(target_id: String) -> Vec<SnapshotEntry> {
    list_snapshots_at(&backup_dir_for(&target_id), &target_id)
}

fn list_snapshots_at(dir: &Path, target_id: &str) -> Vec<SnapshotEntry> {
    if !dir.exists() {
        return vec![];
    }
    let mut entries: Vec<SnapshotEntry> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let fname = e.file_name().into_string().ok()?;
            let parsed = parse_snapshot_name(&fname)?;
            Some(SnapshotEntry {
                target_id: target_id.to_owned(),
                filename: fname.clone(),
                timestamp: parsed.seconds,
                original_name: parsed.original_name.to_owned(),
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    entries
}

/// 从备份文件恢复到目标应用的实际配置路径
#[tauri::command]
pub async fn restore_snapshot(target_id: String, filename: String) -> Result<(), SafeCommandError> {
    let _guard = crate::commands::targets::lock_and_recover_provider_transaction()?;
    restore_snapshot_after_recovery_at(
        &backup_dir_for(&target_id),
        &crate::targets::user_home_dir(),
        &target_id,
        &filename,
        crate::commands::codex_sessions::recover_codex_session_storage,
    )
}

fn restore_snapshot_after_recovery_at(
    backup_dir: &Path,
    home: &Path,
    target_id: &str,
    filename: &str,
    recover: impl FnOnce() -> Result<(), SafeCommandError>,
) -> Result<(), SafeCommandError> {
    recover()?;
    restore_snapshot_at(backup_dir, home, target_id, filename)
}

fn restore_snapshot_at(
    backup_dir: &Path,
    home: &Path,
    target_id: &str,
    filename: &str,
) -> Result<(), SafeCommandError> {
    let parsed = parse_snapshot_name(filename).ok_or_else(SafeCommandError::invalid_request)?;

    let destination = match (target_id, parsed.original_name) {
        ("codex", "auth.json") => home.join(".codex").join("auth.json"),
        ("codex", "config.toml") => home.join(".codex").join("config.toml"),
        ("claude-desktop" | "claude-code", "settings.json") => {
            home.join(".claude").join("settings.json")
        }
        _ => return Err(SafeCommandError::invalid_request()),
    };
    let source = backup_dir.join(filename);
    let metadata =
        fs::symlink_metadata(&source).map_err(|_| SafeCommandError::change_failed(false))?;
    if !metadata.file_type().is_file() {
        return Err(SafeCommandError::invalid_request());
    }
    crate::commands::targets::replace_from_backup(&source, &destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_filename(seconds: u64, nonce: u32, original_name: &str) -> String {
        format_snapshot_name(seconds, nonce, original_name).unwrap()
    }

    #[test]
    fn restore_snapshot_at_only_accepts_known_regular_backups() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("123_auth.json"), b"{\"auth_mode\":\"apikey\"}").unwrap();

        let listed = list_snapshots_at(&backups, "codex");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].timestamp, 123);
        assert_eq!(listed[0].filename, "123_auth.json");
        assert_eq!(listed[0].original_name, "auth.json");

        restore_snapshot_at(&backups, &home, "codex", "123_auth.json").unwrap();
        assert_eq!(
            fs::read(home.join(".codex/auth.json")).unwrap(),
            b"{\"auth_mode\":\"apikey\"}"
        );
        assert!(restore_snapshot_at(&backups, &home, "codex", "../123_auth.json").is_err());
        assert!(restore_snapshot_at(&backups, &home, "codex", "123_unknown.json").is_err());

        let error = restore_snapshot_at(
            Path::new("/Users/alice/.codex/journal"),
            Path::new("/Users/alice"),
            "codex",
            "999_config.toml",
        )
        .unwrap_err();
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains("/Users/alice"));
        assert!(!serialized.contains("config.toml"));
        assert!(!serialized.contains("journal"));
    }

    #[cfg(unix)]
    #[test]
    fn restored_auth_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        fs::create_dir_all(&backups).unwrap();
        let source = backups.join("123_auth.json");
        fs::write(&source, b"{\"OPENAI_API_KEY\":\"fixture\"}").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();

        restore_snapshot_at(&backups, &home, "codex", "123_auth.json").unwrap();
        assert_eq!(
            fs::metadata(home.join(".codex/auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_backup_and_restore_secure_auth_but_preserve_regular_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();
        assert!(
            save_backup_at(&backups, &source_dir.join("missing-auth.json"))
                .unwrap()
                .is_none()
        );

        let auth_source = source_dir.join("auth.json");
        fs::write(&auth_source, b"{\"OPENAI_API_KEY\":\"fixture\"}").unwrap();
        fs::set_permissions(&auth_source, fs::Permissions::from_mode(0o644)).unwrap();
        let auth_backup = save_backup_at(&backups, &auth_source).unwrap().unwrap();
        assert_eq!(
            fs::metadata(&auth_backup).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let auth_target = home.join(".codex/auth.json");
        fs::create_dir_all(auth_target.parent().unwrap()).unwrap();
        fs::write(&auth_target, b"old").unwrap();
        fs::set_permissions(&auth_target, fs::Permissions::from_mode(0o644)).unwrap();
        restore_snapshot_at(
            &backups,
            &home,
            "codex",
            auth_backup.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&auth_target).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let config_source = source_dir.join("config.toml");
        fs::write(&config_source, b"model = \"fixture\"").unwrap();
        fs::set_permissions(&config_source, fs::Permissions::from_mode(0o640)).unwrap();
        let config_backup = save_backup_at(&backups, &config_source).unwrap().unwrap();
        assert_eq!(
            fs::metadata(&config_backup).unwrap().permissions().mode() & 0o777,
            0o640
        );
        restore_snapshot_at(
            &backups,
            &home,
            "codex",
            config_backup.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(home.join(".codex/config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }

    #[test]
    fn same_second_real_saves_are_listed_and_restore_independently() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        let source = temp.path().join("settings.json");
        let seconds = 1_700;
        let time = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(seconds))
            .unwrap();

        fs::write(&source, b"first").unwrap();
        let first = save_backup_at_time(&backups, &source, time)
            .unwrap()
            .unwrap();
        fs::write(&source, b"second").unwrap();
        let second = save_backup_at_time(&backups, &source, time)
            .unwrap()
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"first");
        assert_eq!(fs::read(&second).unwrap(), b"second");

        let listed = list_snapshots_at(&backups, "claude-code");
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|entry| entry.timestamp == seconds));
        assert!(listed
            .iter()
            .any(|entry| entry.filename == first.file_name().unwrap().to_str().unwrap()));
        assert!(listed
            .iter()
            .any(|entry| entry.filename == second.file_name().unwrap().to_str().unwrap()));

        restore_snapshot_at(
            &backups,
            &home,
            "claude-code",
            first.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read(home.join(".claude/settings.json")).unwrap(),
            b"first"
        );
        restore_snapshot_at(
            &backups,
            &home,
            "claude-code",
            second.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read(home.join(".claude/settings.json")).unwrap(),
            b"second"
        );
    }

    #[test]
    fn epoch_and_early_v2_timestamps_are_explicit_and_distinct_from_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let source = temp.path().join("auth.json");
        fs::write(&source, b"epoch").unwrap();
        let epoch = save_backup_at_time(&backups, &source, std::time::UNIX_EPOCH)
            .unwrap()
            .unwrap();
        fs::write(
            backups.join(snapshot_filename(1_700, 0x12ab, "config.toml")),
            b"early",
        )
        .unwrap();
        fs::write(backups.join("1700000000_config.toml"), b"legacy").unwrap();

        let listed = list_snapshots_at(&backups, "codex");
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().any(|entry| entry.filename
            == epoch.file_name().unwrap().to_str().unwrap()
            && entry.timestamp == 0));
        assert!(listed.iter().any(|entry| entry.filename
            == snapshot_filename(1_700, 0x12ab, "config.toml")
            && entry.timestamp == 1_700));
        assert!(listed
            .iter()
            .any(|entry| entry.filename == "1700000000_config.toml"
                && entry.timestamp == 1_700_000_000));
    }

    #[test]
    fn maximum_timestamp_is_self_describing_and_invalid_time_creates_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let source = temp.path().join("auth.json");
        fs::write(&source, b"fixture").unwrap();

        let maximum = snapshot_filename(u64::MAX, u32::MAX, "auth.json");
        assert_eq!(
            parse_snapshot_name(&maximum),
            Some(ParsedSnapshotName {
                seconds: u64::MAX,
                original_name: "auth.json",
            })
        );

        let mut lower = 0u64;
        let mut upper = u64::MAX;
        while lower < upper {
            let span = upper.checked_sub(lower).unwrap();
            let midpoint = lower
                .checked_add(span / 2)
                .and_then(|value| value.checked_add(span % 2))
                .unwrap();
            if std::time::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(midpoint))
                .is_some()
            {
                lower = midpoint;
            } else {
                upper = midpoint.checked_sub(1).unwrap();
            }
        }
        let platform_maximum = snapshot_filename(lower, u32::MAX, "auth.json");
        assert_eq!(
            parse_snapshot_name(&platform_maximum).unwrap().seconds,
            lower
        );
        assert!(!backups.exists());

        let before_epoch = std::time::UNIX_EPOCH
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap();
        let error = save_backup_at_time(&backups, &source, before_epoch).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!backups.exists());
    }

    #[test]
    fn malformed_v2_names_are_never_reinterpreted_as_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        fs::create_dir_all(&backups).unwrap();
        let malformed = [
            "v2_1700_auth.json",
            "v2_1700_1234567_auth.json",
            "v2_1700_1234567G_auth.json",
            "v2_01700_12345678_auth.json",
            "v2_18446744073709551616_12345678_auth.json",
            "v2_1700_12345678_",
        ];
        for filename in malformed {
            fs::write(backups.join(filename), b"invalid").unwrap();
            assert!(parse_snapshot_name(filename).is_none(), "{filename}");
            assert!(restore_snapshot_at(&backups, &home, "codex", filename).is_err());
        }
        assert!(list_snapshots_at(&backups, "codex").is_empty());
    }

    #[test]
    fn occupied_snapshot_file_is_preserved_and_next_candidate_is_used() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let source = temp.path().join("settings.json");
        fs::create_dir_all(&backups).unwrap();
        fs::write(&source, b"new").unwrap();
        let seconds = 1_700_000_000;
        let occupied_nonce = 303;
        let next_nonce = 404;
        let occupied = backups.join(snapshot_filename(seconds, occupied_nonce, "settings.json"));
        fs::write(&occupied, b"occupied").unwrap();

        let created = save_backup_at_with_candidates(
            &backups,
            &source,
            seconds,
            [occupied_nonce, next_nonce],
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(occupied).unwrap(), b"occupied");
        assert_eq!(fs::read(created).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn occupied_snapshot_symlink_and_exhausted_candidates_preserve_sentinel() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let source = temp.path().join("auth.json");
        let sentinel = temp.path().join("sentinel");
        fs::create_dir_all(&backups).unwrap();
        fs::write(&source, b"new").unwrap();
        fs::write(&sentinel, b"sentinel").unwrap();
        let seconds = 1_700_000_000;
        let occupied_nonce = 505;
        let occupied = backups.join(snapshot_filename(seconds, occupied_nonce, "auth.json"));
        symlink(&sentinel, &occupied).unwrap();

        let error = save_backup_at_with_candidates(
            &backups,
            &source,
            seconds,
            std::iter::repeat_n(occupied_nonce, SNAPSHOT_CREATE_ATTEMPTS),
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(fs::symlink_metadata(&occupied)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(sentinel).unwrap(), b"sentinel");
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_write_failures_observe_private_mode_and_clean_only_owned_file() {
        use std::os::unix::fs::PermissionsExt;

        for (index, failed_step) in [
            SnapshotWriteStep::Copy,
            SnapshotWriteStep::Permissions,
            SnapshotWriteStep::FileSync,
            SnapshotWriteStep::ParentSync,
        ]
        .into_iter()
        .enumerate()
        {
            let temp = tempfile::tempdir().unwrap();
            let backups = temp.path().join("backups");
            let source = temp.path().join("auth.json");
            let sentinel = temp.path().join("sentinel");
            fs::create_dir_all(&backups).unwrap();
            fs::write(&source, b"fixture-secret").unwrap();
            fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
            fs::write(&sentinel, b"sentinel").unwrap();
            let seconds = 1_700_000_000;
            let candidate = 600 + index as u32;
            let owned = backups.join(snapshot_filename(seconds, candidate, "auth.json"));

            let result =
                save_backup_at_with_candidates(&backups, &source, seconds, [candidate], |step| {
                    if step != failed_step {
                        return Ok(());
                    }
                    assert_eq!(
                        fs::metadata(&owned).unwrap().permissions().mode() & 0o777,
                        0o600,
                        "{failed_step:?}"
                    );
                    Err(io::Error::other("injected snapshot failure"))
                });
            assert!(result.is_err(), "{failed_step:?}");
            assert!(!owned.exists(), "{failed_step:?}");
            assert_eq!(fs::read(sentinel).unwrap(), b"sentinel");
        }
    }

    #[test]
    fn pending_migration_recovers_before_snapshot_write() {
        let temp = tempfile::tempdir().unwrap();
        let backups = temp.path().join("backups");
        let home = temp.path().join("home");
        let destination = home.join(".codex/config.toml");
        fs::create_dir_all(&backups).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(backups.join("123_config.toml"), b"snapshot").unwrap();
        fs::write(&destination, b"interrupted").unwrap();

        restore_snapshot_after_recovery_at(&backups, &home, "codex", "123_config.toml", || {
            assert_eq!(fs::read(&destination).unwrap(), b"interrupted");
            fs::write(&destination, b"recovered-old").unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"snapshot");
    }
}
