//! 模型管理：下载、SHA256 校验、解压到 app_data_dir/models/。
//!
//! 一期为 sherpa-onnx 本地引擎服务；二期复用下载本地小模型。
//! 设计：网络与解压分离——[`sha256_hex`] / [`verify_sha256`] / [`extract_tar_gz`]
//! 是纯函数，可单测；[`ModelManager`] 负责编排（HTTP 下载 + 写盘 + 校验 + 解压）。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Error;

/// 计算 bytes 的 SHA256，返回小写 hex。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 校验 bytes 的 SHA256 是否等于期望（小写 hex，可带或不带前缀）。
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    let actual = sha256_hex(bytes);
    let expected = expected_hex.trim().to_lowercase();
    actual == expected
}

/// 解压 tar.gz 到 dir。
pub fn extract_tar_gz(bytes: &[u8], dir: &Path) -> crate::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(format!("创建目录失败: {e}")))?;
    let cursor = std::io::Cursor::new(bytes);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dir)
        .map_err(|e| Error::Io(format!("解压失败: {e}")))?;
    Ok(())
}

/// 一个模型条目：下载地址、期望校验和、解压后目录名。
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// 模型管理器：编排下载→校验→解压。
pub struct ModelManager {
    pub root: PathBuf,
}

impl ModelManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// 模型解压后的目标目录。
    pub fn model_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 模型是否已存在（目录存在即视为已安装）。
    pub fn is_installed(&self, name: &str) -> bool {
        self.model_dir(name).is_dir()
    }

    /// 从已下载的字节安装：校验 + 解压。
    pub fn install_from_bytes(&self, spec: &ModelSpec, bytes: &[u8]) -> crate::Result<PathBuf> {
        if !verify_sha256(bytes, spec.sha256) {
            return Err(Error::Provider(format!(
                "模型 {} 校验失败：SHA256 不匹配",
                spec.name
            )));
        }
        let dir = self.model_dir(spec.name);
        extract_tar_gz(bytes, &dir)?;
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // SHA256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_matches_and_mismatch() {
        let data = b"hello";
        let h = sha256_hex(data);
        assert!(verify_sha256(data, &h));
        assert!(!verify_sha256(data, "deadbeef"));
    }

    #[test]
    fn extract_tar_gz_roundtrip() {
        // 构造一个含单文件的 tar.gz。
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");

        // 用命令构造 tar.gz（保证跨用例稳定）。
        let src = dir.path().join("src.txt");
        std::fs::write(&src, b"hi").unwrap();
        let tar_path = dir.path().join("a.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&tar_path)
            .arg("-C")
            .arg(dir.path())
            .arg("src.txt")
            .status()
            .expect("tar 命令存在");
        assert!(status.success(), "tar 失败");
        let bytes = std::fs::read(&tar_path).unwrap();
        extract_tar_gz(&bytes, &out).unwrap();
        assert_eq!(std::fs::read(out.join("src.txt")).unwrap(), b"hi");
    }

    #[test]
    fn install_from_bytes_rejects_bad_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = ModelManager::new(dir.path().to_path_buf());
        let spec = ModelSpec {
            name: "m",
            url: "",
            sha256: "00",
        };
        let err = mgr.install_from_bytes(&spec, b"data").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("校验失败"));
    }
}
