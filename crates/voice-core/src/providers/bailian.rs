//! 阿里云百炼 Protocol A 流式 ASR provider。
//!
//! 架构：`connect()` 建立 WS 并立即把"读循环"spawn 到后台 task，
//! 读循环把服务端 `result-generated` 转 `TranscriptDelta` 推到 channel。
//! session 本身只持有写端（发音频 / finish-task）+ deltas 的 receiver，
//! 因此 `feed`/`finish`/`deltas` 无需 self-move。
//!
//! 测试：`tests/integration_bailian.rs` 起本地 WS mock server 模拟百炼。

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use futures::{SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::bailian_proto::{self, default_params, finish_task, run_task, Event, ResponseEnvelope};
use crate::config::ProviderKind;
use crate::traits::{AsrProvider, AsrSession, AudioFrame, TranscriptDelta, TranscriptKind};
use crate::{Error, ProviderConfig};

const AUTH_HEADER: &str = "Authorization";

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures::stream::SplitSink<WsStream, Message>;
type WsSource = futures::stream::SplitStream<WsStream>;

/// 百炼 provider。无状态，可在多 session 间复用。
pub struct BailianProvider;

#[async_trait]
impl AsrProvider for BailianProvider {
    async fn connect(&self, cfg: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
        if !matches!(cfg.kind, ProviderKind::Bailian) {
            return Err(Error::Config(format!(
                "BailianProvider 收到非 bailian 配置: {:?}",
                cfg.kind
            )));
        }
        cfg.validate()?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let params = default_params();

        // 1) 建立 WS（带 Authorization）。
        let ws = connect_ws(cfg).await?;
        let (mut tx, mut rx) = ws.split();

        // 2) 发 run-task。
        let req = run_task(&task_id, &cfg.model, params);
        tx.send(Message::Text(bailian_proto::to_json(&req)?))
            .await
            .map_err(|e| Error::Protocol(format!("发送 run-task 失败: {e}")))?;

        // 3) 等 task-started（顺带丢弃可能早到的 result）。
        loop {
            match recv_response(&mut rx).await? {
                Some(env) if env.header.event == Event::TaskStarted => break,
                Some(env) if env.header.event == Event::TaskFailed => {
                    return Err(Error::Provider(format!(
                        "run-task 失败: {} {}",
                        env.header.error_code.unwrap_or_default(),
                        env.header.error_message.unwrap_or_default()
                    )));
                }
                Some(_) => {}
                None => return Err(Error::Provider("连接在 task-started 前关闭".into())),
            }
        }

        // 4) spawn 读循环，把响应转 deltas 推 channel。
        let (dtx, drx) = mpsc::unbounded_channel::<crate::Result<TranscriptDelta>>();
        tokio::spawn(read_loop(rx, dtx));

        Ok(Box::new(BailianSession {
            ws_tx: tx,
            task_id,
            deltas_rx: Some(drx),
            finished: false,
        }))
    }
}

/// 读循环：把服务端 result-generated 转 delta；终态/断连时关闭 channel。
async fn read_loop(mut rx: WsSource, tx: mpsc::UnboundedSender<crate::Result<TranscriptDelta>>) {
    let mut sentence_index: u32 = 0;
    loop {
        match recv_response(&mut rx).await {
            Ok(Some(env)) => match env.header.event {
                Event::ResultGenerated => {
                    if let Some(out) = env.payload.output.and_then(|o| o.sentence) {
                        if out.heartbeat {
                            continue;
                        }
                        let idx = if out.sentence_end {
                            let i = sentence_index;
                            sentence_index += 1;
                            i
                        } else {
                            sentence_index
                        };
                        let kind = if out.sentence_end {
                            TranscriptKind::Final
                        } else {
                            TranscriptKind::Partial
                        };
                        if tx
                            .send(Ok(TranscriptDelta {
                                kind,
                                text: out.text,
                                sentence_index: idx,
                            }))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Event::TaskFinished | Event::TaskFailed => break,
                Event::TaskStarted => {}
            },
            Ok(None) => break, // 连接关闭
            Err(e) => {
                let _ = tx.send(Err(e));
                break;
            }
        }
    }
    // tx drop 后，UnboundedReceiverStream 自然结束。
}

async fn recv_response(rx: &mut WsSource) -> crate::Result<Option<ResponseEnvelope>> {
    loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) => {
                return Ok(Some(bailian_proto::parse_response(&t)?));
            }
            Some(Ok(Message::Close(_))) | None => return Ok(None),
            Some(Ok(
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_),
            )) => continue,
            Some(Err(e)) => return Err(Error::Protocol(format!("读取失败: {e}"))),
        }
    }
}

/// 测试云端连接：建立 WS → 发 run-task → 等 task-started → 发 finish-task。
/// 成功返回 Ok(())，失败返回具体错误。
pub async fn test_connection(cfg: &ProviderConfig) -> crate::Result<String> {
    cfg.validate()?;
    let ws = connect_ws(cfg).await?;
    let (mut tx, mut rx) = ws.split();

    let task_id = uuid::Uuid::new_v4().to_string();
    let req = run_task(&task_id, &cfg.model, default_params());
    tx.send(Message::Text(bailian_proto::to_json(&req)?))
        .await
        .map_err(|e| Error::Protocol(format!("发送 run-task 失败: {e}")))?;

    // 等 task-started（超时 10s）。
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                return Err(Error::Protocol("连接测试超时（10s 内未收到 task-started）".into()));
            }
            result = recv_response(&mut rx) => {
                match result? {
                    Some(env) if env.header.event == Event::TaskStarted => break,
                    Some(env) if env.header.event == Event::TaskFailed => {
                        return Err(Error::Provider(format!(
                            "服务端拒绝: {} {}",
                            env.header.error_code.unwrap_or_default(),
                            env.header.error_message.unwrap_or_default()
                        )));
                    }
                    Some(_) => {}
                    None => return Err(Error::Provider("连接在 task-started 前关闭".into())),
                }
            }
        }
    }

    // 正常关闭。
    let _ = tx
        .send(Message::Text(bailian_proto::to_json(&finish_task(
            &task_id,
        ))?))
        .await;
    let _ = tx.close().await;
    Ok(format!("连接成功！模型 {} 已就绪", cfg.model))
}

async fn connect_ws(cfg: &ProviderConfig) -> crate::Result<WsStream> {
    let mut req = cfg
        .base_url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::Protocol(format!("构造 WS 请求失败: {e}")))?;
    let headers = req.headers_mut();
    headers.insert(
        AUTH_HEADER,
        format!("Bearer {}", cfg.api_key)
            .parse()
            .map_err(|e| Error::Protocol(format!("非法 api_key: {e}")))?,
    );
    headers.insert("User-Agent", "openIME/0.1".parse().unwrap());
    headers.insert("X-DashScope-DataInspection", "enable".parse().unwrap());

    let (ws, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| Error::Protocol(format!("WebSocket 连接失败: {e}")))?;
    Ok(ws)
}

/// 一次百炼转写会话。读循环在 connect 时已 spawn 到后台，结束时关闭 deltas 流。
pub struct BailianSession {
    ws_tx: WsSink,
    task_id: String,
    deltas_rx: Option<mpsc::UnboundedReceiver<crate::Result<TranscriptDelta>>>,
    finished: bool,
}

impl AsrSession for BailianSession {
    fn feed(
        &mut self,
        frame: &AudioFrame,
    ) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + '_>> {
        let bytes = frame.bytes.clone();
        Box::pin(async move {
            self.ws_tx
                .send(Message::Binary(bytes))
                .await
                .map_err(|e| Error::Protocol(format!("发送音频失败: {e}")))
        })
    }

    fn finish(&mut self) -> Pin<Box<dyn Future<Output = crate::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if !self.finished {
                self.finished = true;
                let req = finish_task(&self.task_id);
                let json = bailian_proto::to_json(&req)?;
                self.ws_tx
                    .send(Message::Text(json))
                    .await
                    .map_err(|e| Error::Protocol(format!("发送 finish-task 失败: {e}")))?;
            }
            Ok(())
        })
    }

    fn deltas(&mut self) -> Pin<Box<dyn Stream<Item = crate::Result<TranscriptDelta>> + Send>> {
        let rx = self.deltas_rx.take().expect("deltas() 只能调用一次");
        Box::pin(UnboundedReceiverStream::new(rx))
    }
}
