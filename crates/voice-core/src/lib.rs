//! voice-core: 语音输入引擎核心库。
//!
//! 不依赖 Tauri，所有逻辑以可 mock 的 trait 暴露，便于 `cargo test`。
//! 一期组成：
//! - [`traits`]：四个核心抽象 AudioSource / AsrProvider / TextInserter / HistoryStore
//! - [`config`]：ProviderConfig、AppConfig
//! - [`store`]：Sqlite 历史记录（M1）
//! - [`bailian_proto`]：百炼 Protocol A 协议帧（M2）
//! - [`providers::bailian`]：百炼 WebSocket provider（M2）
//! - [`audio`]：cpal 采集 + 重采样（M3）
//! - [`providers::sherpa`]：本地 sherpa-onnx provider（M4）
//! - [`insert`]：enigo 文本插入（M5）
//! - [`pipeline`]：端到端编排（M5）

pub mod audio;
pub mod bailian_proto;
pub mod config;
pub mod insert;
pub mod model_download;
pub mod model_mgr;
pub mod permissions;
pub mod pipeline;
pub mod providers;
pub mod store;
pub mod traits;

pub use config::{AppConfig, ProviderConfig, ProviderKind};
pub use insert::EnigoInserter;
pub use model_download::{
    install_local_engine, is_local_engine_installed, is_local_engine_installed_for,
    local_model_files, local_model_files_for, missing_files, missing_files_for, DownloadProgress,
    SENSEVOICE_MODEL_NAME, SHERPA_MODEL_NAME, VAD_DIR,
};
pub use model_mgr::ModelManager;
pub use providers::bailian::test_connection;
pub use providers::sherpa::{SherpaModelPaths, SherpaProvider};
pub use providers::RoutingProvider;
pub use store::{Hotword, SqliteStore};
pub use traits::{
    AsrProvider, AsrSession, AudioFormat, AudioFrame, AudioSource, HistoryStore, SessionSummary,
    TextInserter, TranscriptDelta, TranscriptKind, UtteranceRecord,
};

use thiserror::Error;

/// 库统一错误类型。各 provider/store 在转换为自己的领域错误后归并到这里。
#[derive(Debug, Error)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("I/O 错误: {0}")]
    Io(String),
    #[error("音频错误: {0}")]
    Audio(String),
    #[error("ASR provider 错误: {0}")]
    Provider(String),
    #[error("协议错误: {0}")]
    Protocol(String),
    #[error("存储错误: {0}")]
    Store(String),
    #[error("权限缺失: {0}")]
    Permission(String),
    #[error("文本插入失败: {0}")]
    Insert(String),
}

pub type Result<T> = std::result::Result<T, Error>;
