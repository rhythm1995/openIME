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
        MOCK.with(|m| {
            m.borrow_mut()
                .insert("polish_cloud".to_string(), key.to_string())
        });
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

/// 删除某 provider 的 api_key（按 provider 索引）。用户清空配置时调用——
/// 不删的话 load_config 重启时会回填旧 key（与 JSON 状态不一致）。
pub fn delete_provider_key(index: usize) -> Result<(), String> {
    let username = format!("provider_{index}");
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow_mut().remove(&username));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, &username).map_err(|e| e.to_string())?;
        entry.delete_credential().map_err(|e| e.to_string())
    }
}

/// 删除云端润色 API key（username=`polish_cloud`）。用户清空配置时调用——
/// 不删的话 load_config 重启时会回填旧 key，且「endpoint 空 + key 非空」
/// 会让 check_cloud_llm 拒绝后续所有保存。
pub fn delete_polish_key() -> Result<(), String> {
    #[cfg(test)]
    {
        MOCK.with(|m| m.borrow_mut().remove("polish_cloud"));
        Ok(())
    }
    #[cfg(not(test))]
    {
        let entry = keyring::Entry::new(SERVICE, "polish_cloud").map_err(|e| e.to_string())?;
        entry.delete_credential().map_err(|e| e.to_string())
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

    #[test]
    fn delete_roundtrip() {
        MOCK.with(|m| m.borrow_mut().clear());
        // provider 键删除。
        store_provider_key(0, "sk-secret-0").unwrap();
        delete_provider_key(0).unwrap();
        assert_eq!(fetch_provider_key(0), None);
        // 幂等：不存在的条目删除不报错。
        delete_provider_key(0).unwrap();
        // 云端润色键删除。
        store_polish_key("sk-polish").unwrap();
        delete_polish_key().unwrap();
        assert_eq!(fetch_polish_key(), None);
        delete_polish_key().unwrap();
    }

    /// 手动探针：绕过 mock 直连真实 keychain（验证写入是否被系统拒绝）。
    /// 运行：cargo test -p openime --lib credentials::tests::keychain_probe -- --ignored
    #[test]
    #[ignore = "manual: 真实 keychain 探针，按需运行"]
    fn keychain_probe() {
        let entry = keyring::Entry::new("com.openime.desktop.probe", "probe").unwrap();
        entry.set_password("probe-value").unwrap();
        assert_eq!(entry.get_password().unwrap(), "probe-value");
        entry.delete_credential().ok();
    }
}
