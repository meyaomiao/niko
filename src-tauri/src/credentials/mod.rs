use keyring::Entry;

const SERVICE: &str = "win.momotoken.niko";

pub struct CredentialStore;

impl CredentialStore {
    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        Entry::new(SERVICE, account)
            .map_err(|e| e.to_string())?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }

    pub fn get(account: &str) -> Result<String, String> {
        Entry::new(SERVICE, account)
            .map_err(|e| e.to_string())?
            .get_password()
            .map_err(|e| e.to_string())
    }

    pub fn delete(account: &str) -> Result<(), String> {
        let entry = Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // 不存在视为删除成功
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
