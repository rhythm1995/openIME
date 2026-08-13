//! 凭据安全存储（H2）：API key 存系统钥匙串（macOS Keychain / Windows 凭据管理器），
//! 不落明文 JSON。测试用线程本地 HashMap mock（CI 无 keychain）。
//!
//! 参考思路：OpenLess `commands/credentials.rs` + `persistence.rs::CredentialsVault`。

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::HashMap;

#[cfg(not(test))]
const SERVICE: &str = "com.openime.desktop";

#[cfg(test)]
thread_local! {
    static MOCK: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// 存某 provider 的 api_key 到 keychain（按 provider 索引）。
pub fn store_provider_key(index: usize, key: &str) -> Result<(), String> {
    let username = format!("provider_{index}");
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow_mut().insert(username, key.to_string()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, &username).map_err(|e| e.to_string())?;
        entry.set_password(key).map_err(|e| e.to_string())
    }
}

/// 从 keychain 取某 provider 的 api_key；无则 None。
pub fn fetch_provider_key(index: usize) -> Option<String> {
    let username = format!("provider_{index}");
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow().get(&username).cloned())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, &username).ok()?;
        entry.get_password().ok()
    }
}

/// 存云端润色 API key 到 keychain（username=`polish_cloud`）。PR1：不再落明文 JSON。
pub fn store_polish_key(key: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow_mut().insert("polish_cloud".to_string(), key.to_string()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, "polish_cloud").map_err(|e| e.to_string())?;
        entry.set_password(key).map_err(|e| e.to_string())
    }
}

/// 从 keychain 取云端润色 API key；无则 None。
pub fn fetch_polish_key() -> Option<String> {
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow().get("polish_cloud").cloned())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, "polish_cloud").ok()?;
        entry.get_password().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_fetch_roundtrip() {
        MOCK.with(|m| m.borrow_mut().clear());
        store_provider_key(0, "sk-secret-0").unwrap();
        store_provider_key(1, "sk-secret-1").unwrap();
        assert_eq!(fetch_provider_key(0), Some("sk-secret-0".into()));
        assert_eq!(fetch_provider_key(1), Some("sk-secret-1".into()));
        assert_eq!(fetch_provider_key(2), None);
    }

    #[test]
    fn polish_key_roundtrip() {
        MOCK.with(|m| m.borrow_mut().clear());
        assert_eq!(fetch_polish_key(), None);
        store_polish_key("sk-polish").unwrap();
        assert_eq!(fetch_polish_key(), Some("sk-polish".into()));
    }
}
