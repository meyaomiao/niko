/// Keychain / secret-service 封装占位。
/// E2-5 中用 `keyring` crate 实现跨平台凭证存储，此处为 stub。
pub struct CredentialStore {
    service: String,
}

impl CredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let _ = (account, secret, &self.service);
        Err("not implemented — E2-5".into())
    }

    pub fn get(&self, account: &str) -> Result<String, String> {
        let _ = (account, &self.service);
        Err("not implemented — E2-5".into())
    }

    pub fn delete(&self, account: &str) -> Result<(), String> {
        let _ = (account, &self.service);
        Err("not implemented — E2-5".into())
    }
}
