//! 应用全局状态：DB + 配置 + pipeline（懒初始化，避免启动期 enigo 等副作用导致 abort）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;
use voice_core::pipeline::{Pipeline, PipelineDeps};
use voice_core::{
    AppConfig, EnigoInserter, Error, HistoryStore, RoutingProvider, SqliteStore, TextInserter,
};

pub const CONFIG_KEY: &str = "app_config";

/// 进程级共享状态。
pub struct AppState {
    pub store: Arc<SqliteStore>,
    pub config: Arc<RwLock<AppConfig>>,
    pub sherpa_root: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// pipeline 懒初始化：首次录音时建立（含 EnigoInserter，可能在无辅助功能权限时失败）。
    pipeline: RwLock<Option<Arc<Pipeline>>>,
    /// 当前是否正在录音（前端/overlay 查询用）。
    pub recording: Arc<RwLock<bool>>,
    /// 录音停止标志：toggle 再次调用时置 true，录音循环看到后停止。
    pub stop_flag: Arc<AtomicBool>,
    /// 本地模型下载中（防并发重复下载）。
    pub model_downloading: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(
        store: SqliteStore,
        sherpa_root: Option<(std::path::PathBuf, std::path::PathBuf)>,
    ) -> Result<Self, Error> {
        let mut config = load_config(&store)?.unwrap_or_default();
        // 迁移：旧版本快捷键硬编码 Alt+Shift+D、设置项保存后并不生效，
        // 该值不代表用户主动选择，统一迁到新默认 Fn。
        if config.hotkey == "Alt+Shift+D" {
            config.hotkey = "Fn".to_string();
        }
        // 迁移：旧版本默认 bailian + 空 base_url，新版本默认 sherpa。
        // 如果用户从未配置云端地址，把 provider 切到 sherpa（本地引擎）。
        if let Some(p) = config.providers.get_mut(config.active_provider) {
            if p.kind == voice_core::ProviderKind::Bailian && p.base_url.trim().is_empty() {
                p.kind = voice_core::ProviderKind::Sherpa;
                p.model = "sherpa-onnx-streaming-paraformer-bilingual-zh-en".to_string();
            }
        }
        let store_arc: Arc<SqliteStore> = Arc::new(store);
        Ok(Self {
            store: store_arc,
            config: Arc::new(RwLock::new(config)),
            sherpa_root,
            pipeline: RwLock::new(None),
            recording: Arc::new(RwLock::new(false)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            model_downloading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 本地模型根目录（下载/安装本地引擎用）。
    pub fn model_root(&self) -> Option<std::path::PathBuf> {
        self.sherpa_root.as_ref().map(|(root, _)| root.clone())
    }

    /// 取得 pipeline；首次调用时构造。enigo 初始化失败则返回错误（不会 abort 启动）。
    pub async fn pipeline(&self) -> Result<Arc<Pipeline>, Error> {
        {
            let guard = self.pipeline.read().await;
            if let Some(p) = guard.as_ref() {
                return Ok(p.clone());
            }
        }
        let mut guard = self.pipeline.write().await;
        // double-check
        if let Some(p) = guard.as_ref() {
            return Ok(p.clone());
        }
        let provider: Arc<dyn voice_core::AsrProvider> = Arc::new(RoutingProvider {
            sherpa_root: self.sherpa_root.clone(),
        });
        let inserter: Arc<dyn TextInserter> = Arc::new(EnigoInserter::new()?);
        let deps = PipelineDeps {
            provider,
            inserter,
            store: self.store.clone() as Arc<dyn HistoryStore>,
        };
        let p = Arc::new(Pipeline::new(deps));
        *guard = Some(p.clone());
        Ok(p)
    }

    /// 请求停止录音（置停止标志）。
    pub fn request_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn clear_stop(&self) {
        self.stop_flag.store(false, Ordering::SeqCst);
    }
}

pub fn load_config(store: &SqliteStore) -> Result<Option<AppConfig>, Error> {
    match store.get_setting(CONFIG_KEY)? {
        Some(json) => {
            let cfg = serde_json::from_str(&json)
                .map_err(|e| Error::Store(format!("解析 app_config 失败: {e}")))?;
            Ok(Some(cfg))
        }
        None => Ok(None),
    }
}

pub fn save_config(store: &SqliteStore, cfg: &AppConfig) -> Result<(), Error> {
    let json = serde_json::to_string(cfg)
        .map_err(|e| Error::Store(format!("序列化 app_config 失败: {e}")))?;
    store.set_setting(CONFIG_KEY, &json)?;
    Ok(())
}
