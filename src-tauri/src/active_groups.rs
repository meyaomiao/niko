use crate::targets::TargetConfigObservation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const ACTIVE_GROUP_RECORD_VERSION: u16 = 1;
pub(crate) const ACTIVE_GROUP_STATUS_VERSION: u16 = 1;

const ACTIVE_GROUP_DIRECTORY: &str = "active-groups";
const RECORD_GROUP_MAX_CHARS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveGroupRecord {
    pub(crate) version: u16,
    pub(crate) target_id: String,
    pub(crate) group: String,
    pub(crate) config_digest: String,
    pub(crate) enabled_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActiveGroupState {
    Active,
    NotNiko,
    Changed,
    Unknown,
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ActiveGroupStatus {
    pub(crate) version: u16,
    pub(crate) target_id: String,
    pub(crate) status: ActiveGroupState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveSelection {
    pub(crate) status: ActiveGroupState,
    pub(crate) group: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RecordRead {
    Missing,
    Valid(ActiveGroupRecord),
    Invalid,
}

struct OwnedTemporary {
    path: PathBuf,
    file: Option<File>,
}

impl OwnedTemporary {
    fn create(parent: &Path) -> io::Result<Self> {
        for _ in 0..32 {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let path = parent.join(format!(".niko-active-group-{suffix}.tmp"));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "active group temporary names are occupied",
        ))
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("active group temporary is open")
    }

    fn file(&self) -> &File {
        self.file.as_ref().expect("active group temporary is open")
    }

    fn close(&mut self) {
        self.file.take();
    }
}

impl Drop for OwnedTemporary {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn record_root(home: &Path) -> PathBuf {
    home.join(".niko").join(ACTIVE_GROUP_DIRECTORY)
}

pub(crate) fn record_path(home: &Path, target_id: &str) -> io::Result<PathBuf> {
    if !matches!(target_id, "codex" | "claude-desktop") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown active group target",
        ));
    }
    Ok(record_root(home).join(format!("{}.json", target_id)))
}

pub(crate) fn record_for(
    target_id: &str,
    group: &str,
    config_digest: String,
) -> io::Result<ActiveGroupRecord> {
    let record = ActiveGroupRecord {
        version: ACTIVE_GROUP_RECORD_VERSION,
        target_id: target_id.to_owned(),
        group: group.to_owned(),
        config_digest: bind_config_digest(group, &config_digest),
        enabled_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "clock before epoch"))?
            .as_secs(),
    };
    validate_record(&record, target_id)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid active group record"))?;
    Ok(record)
}

pub(crate) fn config_digest(
    target_id: &str,
    endpoint: &str,
    auth_style: &str,
    model: Option<&str>,
    credential: &str,
) -> String {
    digest_values(&[
        target_id,
        endpoint,
        auth_style,
        model.unwrap_or(""),
        credential,
    ])
}

pub(crate) fn bind_config_digest(group: &str, config_digest: &str) -> String {
    digest_values(&[group, config_digest])
}

fn digest_values(values: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_private_directory(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "active group directory is not a regular directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn sync_parent(path: &Path) -> io::Result<()> {
    crate::codex_sessions::sync_parent(path)
        .map_err(|_| io::Error::other("active group parent sync failed"))
}

fn ensure_record_root(home: &Path) -> io::Result<PathBuf> {
    let control = home.join(".niko");
    if !is_private_directory(&control)? {
        fs::create_dir(&control)?;
    }
    #[cfg(unix)]
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700))?;
    sync_parent(&control)?;

    let root = control.join(ACTIVE_GROUP_DIRECTORY);
    if !is_private_directory(&root)? {
        fs::create_dir(&root)?;
    }
    #[cfg(unix)]
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    sync_parent(&root)?;
    Ok(root)
}

fn record_exists_state(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "active group record is not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_record_at(home: &Path, target_id: &str) -> RecordRead {
    let Ok(path) = record_path(home, target_id) else {
        return RecordRead::Invalid;
    };
    let root = record_root(home);
    match is_private_directory(&root) {
        Ok(true) => {}
        Ok(false) => return RecordRead::Missing,
        Err(_) => return RecordRead::Invalid,
    }
    match record_exists_state(&path) {
        Ok(false) => RecordRead::Missing,
        Ok(true) => {
            let Ok(bytes) = fs::read(&path) else {
                return RecordRead::Invalid;
            };
            match serde_json::from_slice::<ActiveGroupRecord>(&bytes) {
                Ok(record) if validate_record(&record, target_id).is_ok() => {
                    RecordRead::Valid(record)
                }
                _ => RecordRead::Invalid,
            }
        }
        Err(_) => RecordRead::Invalid,
    }
}

pub(crate) fn write_record_at(home: &Path, record: &ActiveGroupRecord) -> io::Result<()> {
    validate_record(record, &record.target_id)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid active group record"))?;
    let root = ensure_record_root(home)?;
    let path = record_path(home, &record.target_id)?;
    if record_exists_state(&path)? {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "active group record is a symlink",
            ));
        }
    }

    let mut temporary = OwnedTemporary::create(&root)?;
    let mut bytes = serde_json::to_vec(record)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "record serialization failed"))?;
    bytes.push(b'\n');
    temporary.file_mut().write_all(&bytes)?;
    temporary.file().sync_all()?;
    sync_parent(&temporary.path)?;
    temporary.close();
    crate::codex_sessions::atomic_replace_file(&temporary.path, &path, None)
        .map_err(|_| io::Error::other("active group record replacement failed"))?;
    sync_parent(&path)
}

pub(crate) fn clear_record_at(home: &Path, target_id: &str) -> io::Result<()> {
    let path = record_path(home, target_id)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::remove_file(&path)?;
            sync_parent(&path)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "active group record is not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_record(record: &ActiveGroupRecord, target_id: &str) -> Result<(), ()> {
    if record.version != ACTIVE_GROUP_RECORD_VERSION
        || record.target_id != target_id
        || record.group.is_empty()
        || record.group.chars().count() > RECORD_GROUP_MAX_CHARS
        || record.enabled_at == 0
        || record.config_digest.len() != 64
        || !record
            .config_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(());
    }
    Ok(())
}

fn active_status(
    target_id: &str,
    group: Option<String>,
    status: ActiveGroupState,
) -> ActiveGroupStatus {
    ActiveGroupStatus {
        version: ACTIVE_GROUP_STATUS_VERSION,
        target_id: target_id.to_owned(),
        status,
        group,
    }
}

fn group_is_available(group: &str, available_groups: Option<&[String]>) -> bool {
    available_groups.is_none_or(|groups| groups.iter().any(|item| item == group))
}

pub(crate) fn detect_effective_selection_at(
    home: &Path,
    target_id: &str,
    available_groups: Option<&[String]>,
) -> EffectiveSelection {
    detect_selection_at(home, target_id, available_groups, true)
}

fn detect_selection_at(
    home: &Path,
    target_id: &str,
    available_groups: Option<&[String]>,
    require_model: bool,
) -> EffectiveSelection {
    let record = read_record_at(home, target_id);
    if matches!(&record, RecordRead::Invalid) {
        return EffectiveSelection { status: ActiveGroupState::Unknown, group: None, model: None };
    }

    let observation = crate::targets::observe_active_config_at(target_id, home);
    match record {
        RecordRead::Missing => match observation {
            TargetConfigObservation::Other => {
                EffectiveSelection { status: ActiveGroupState::NotNiko, group: None, model: None }
            }
            TargetConfigObservation::Unreadable => {
                EffectiveSelection { status: ActiveGroupState::Unreadable, group: None, model: None }
            }
            TargetConfigObservation::Matchable(_) | TargetConfigObservation::Ambiguous => {
                EffectiveSelection { status: ActiveGroupState::Unknown, group: None, model: None }
            }
        },
        RecordRead::Valid(record) => {
            if !group_is_available(&record.group, available_groups) {
                return EffectiveSelection { status: ActiveGroupState::Unknown, group: None, model: None };
            }
            match observation {
                TargetConfigObservation::Unreadable => {
                    EffectiveSelection { status: ActiveGroupState::Unreadable, group: None, model: None }
                }
                TargetConfigObservation::Matchable(config) if require_model && config.model.is_none() => {
                    EffectiveSelection { status: ActiveGroupState::Unknown, group: None, model: None }
                }
                TargetConfigObservation::Matchable(config)
                    if bind_config_digest(&record.group, &config.digest) == record.config_digest =>
                {
                    EffectiveSelection {
                        status: ActiveGroupState::Active,
                        group: Some(record.group),
                        model: config.model,
                    }
                }
                TargetConfigObservation::Matchable(_)
                | TargetConfigObservation::Other
                | TargetConfigObservation::Ambiguous => {
                    EffectiveSelection { status: ActiveGroupState::Changed, group: None, model: None }
                }
            }
        }
        RecordRead::Invalid => EffectiveSelection { status: ActiveGroupState::Unknown, group: None, model: None },
    }
}

pub(crate) fn detect_active_group_at(
    home: &Path,
    target_id: &str,
    available_groups: Option<&[String]>,
) -> ActiveGroupStatus {
    let selection = detect_selection_at(home, target_id, available_groups, false);
    active_status(target_id, selection.group, selection.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_home(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn record(target_id: &str, group: &str, digest: &str) -> ActiveGroupRecord {
        ActiveGroupRecord {
            version: ACTIVE_GROUP_RECORD_VERSION,
            target_id: target_id.to_owned(),
            group: group.to_owned(),
            config_digest: digest.to_owned(),
            enabled_at: 1,
        }
    }

    #[test]
    fn record_is_versioned_atomic_and_private() {
        let home = fixture_home("record_atomic");
        let old = record("codex", "A", &"a".repeat(64));
        let new = record("codex", "B", &"b".repeat(64));
        write_record_at(&home, &old).unwrap();
        write_record_at(&home, &new).unwrap();

        assert_eq!(read_record_at(&home, "codex"), RecordRead::Valid(new));
        assert!(!record_root(&home).join(".niko-active-group").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(record_root(&home))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(record_path(&home, "codex").unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn corrupt_and_unknown_records_fail_closed() {
        let home = fixture_home("record_corrupt");
        let path = record_path(&home, "codex").unwrap();
        fs::create_dir_all(record_root(&home)).unwrap();
        fs::write(&path, br#"{"version":99,"target_id":"codex"}"#).unwrap();
        assert_eq!(read_record_at(&home, "codex"), RecordRead::Invalid);
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_record_at(&home, "codex"), RecordRead::Invalid);
    }

    #[test]
    fn clear_record_is_idempotent_and_does_not_leave_secret_data() {
        let home = fixture_home("record_clear");
        let secret = "fixture-secret-do-not-persist";
        let written = record(
            "codex",
            "A",
            &config_digest("codex", "https://example", "openai", None, secret),
        );
        write_record_at(&home, &written).unwrap();
        let bytes = fs::read(record_path(&home, "codex").unwrap()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        clear_record_at(&home, "codex").unwrap();
        clear_record_at(&home, "codex").unwrap();
        assert_eq!(read_record_at(&home, "codex"), RecordRead::Missing);
    }
}
