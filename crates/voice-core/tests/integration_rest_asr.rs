//! REST ASR provider 集成测试：起本地 HTTP mock server，
//! 验证 openai_asr（/audio/transcriptions）与 multimodal_asr（/chat/completions）
//! 全流程：feed PCM → finish 时 POST（base64 WAV）→ 解析响应 → 产 final delta。
//! 覆盖：成功（string content）、成功（array content）、HTTP 错误返回 Err。

use std::sync::{Arc, Mutex as StdMutex};

use futures::StreamExt;
use tokio::sync::oneshot;
use voice_core::providers::multimodal_asr::MultimodalAsrProvider;
use voice_core::providers::openai_asr::OpenAiAsrProvider;
use voice_core::traits::{AsrProvider, AsrSession, AudioFormat, AudioFrame, TranscriptKind};
use voice_core::{ProviderConfig, ProviderKind};

/// mock server 捕获的请求：(路径, body)。
type Captured = Arc<StdMutex<Option<(String, String)>>>;

/// 最小 HTTP server：接受 1 个请求，回 status + JSON body，捕获 (path, body)。
async fn serve_http(
    tx: oneshot::Sender<String>,
    status: u16,
    resp_body: &'static str,
    captured: Captured,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tx.send(addr.to_string()).unwrap();

    let (mut sock, _) = listener.accept().await.unwrap();
    // 读请求：先读头，再按 Content-Length 读完 body。
    let mut buf = vec![0u8; 65536];
    let mut req = Vec::new();
    let header_end = loop {
        let n = sock.read(&mut buf).await.unwrap();
        if n == 0 {
            break None;
        }
        req.extend_from_slice(&buf[..n]);
        if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(pos + 4);
        }
        if req.len() > 1_000_000 {
            break None;
        }
    };
    let header_end = header_end.unwrap_or(req.len());
    let head = String::from_utf8_lossy(&req[..header_end]).to_string();
    let content_len = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while req.len() < header_end + content_len {
        let n = sock.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        req.extend_from_slice(&buf[..n]);
    }
    let path = head
        .lines()
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_string();
    let body = String::from_utf8_lossy(&req[header_end..header_end + content_len]).to_string();
    *captured.lock().unwrap() = Some((path, body));

    let status_text = if status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    let resp = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
        resp_body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
}

fn test_frame() -> AudioFrame {
    AudioFrame::new(AudioFormat::PCM_16K_MONO_S16LE, vec![0u8; 640]) // 20ms
}

fn rest_cfg(kind: ProviderKind, base_url: &str) -> ProviderConfig {
    ProviderConfig {
        kind,
        base_url: base_url.to_string(),
        api_key: "sk-test".into(),
        model: "test-asr-model".into(),
        vocabulary_id: None,
        language: None,
    }
}

/// 通用：跑一次 REST 会话，收集 deltas 里的 final 文本。
async fn run_session(mut session: Box<dyn AsrSession>) -> Vec<String> {
    session.feed(&test_frame()).await.unwrap();
    session.finish().await.unwrap();
    let mut deltas = session.deltas();
    let mut out = Vec::new();
    while let Some(Ok(d)) = deltas.next().await {
        if d.kind == TranscriptKind::Final {
            out.push(d.text);
        }
    }
    out
}

#[tokio::test]
async fn openai_asr_full_flow() {
    let captured: Captured = Arc::new(StdMutex::new(None));
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_http(
        tx,
        200,
        r#"{"text":"你好世界","usage":{"seconds":1}}"#,
        captured.clone(),
    ));
    let addr = rx.await.unwrap();

    let provider = OpenAiAsrProvider;
    let session = provider
        .connect(&rest_cfg(
            ProviderKind::OpenAiAsr,
            &format!("http://{addr}"),
        ))
        .await
        .expect("connect");
    let finals = run_session(session).await;
    assert_eq!(finals, vec!["你好世界".to_string()]);

    // 断言请求：路径 + body 里的 model / input_audio.data(base64) / format。
    let (path, body) = captured.lock().unwrap().clone().expect("捕获到请求");
    assert_eq!(path, "/audio/transcriptions");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["model"], "test-asr-model");
    assert_eq!(v["input_audio"]["format"], "wav");
    assert!(
        v["input_audio"]["data"].as_str().unwrap().len() > 44 * 4 / 3,
        "base64 WAV 数据应非空"
    );
}

#[tokio::test]
async fn openai_asr_http_error_returns_err() {
    let captured: Captured = Arc::new(StdMutex::new(None));
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_http(tx, 500, r#"{"error":"boom"}"#, captured));
    let addr = rx.await.unwrap();

    let provider = OpenAiAsrProvider;
    let mut session = provider
        .connect(&rest_cfg(
            ProviderKind::OpenAiAsr,
            &format!("http://{addr}"),
        ))
        .await
        .expect("connect");
    session.feed(&test_frame()).await.unwrap();
    let err = session.finish().await;
    assert!(err.is_err(), "HTTP 500 应返回 Err，got {err:?}");
}

#[tokio::test]
async fn multimodal_asr_string_content() {
    let captured: Captured = Arc::new(StdMutex::new(None));
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_http(
        tx,
        200,
        r#"{"choices":[{"message":{"content":"你好世界"}}]}"#,
        captured.clone(),
    ));
    let addr = rx.await.unwrap();

    let provider = MultimodalAsrProvider;
    let session = provider
        .connect(&rest_cfg(
            ProviderKind::MultimodalAsr,
            &format!("http://{addr}"),
        ))
        .await
        .expect("connect");
    let finals = run_session(session).await;
    assert_eq!(finals, vec!["你好世界".to_string()]);

    let (path, body) = captured.lock().unwrap().clone().expect("捕获到请求");
    assert_eq!(path, "/chat/completions");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["messages"][0]["content"][0]["type"], "input_audio");
    assert!(v["messages"][0]["content"][0]["input_audio"]["data"]
        .as_str()
        .unwrap()
        .starts_with("data:audio/wav;base64,"));
}

#[tokio::test]
async fn multimodal_asr_array_content() {
    let captured: Captured = Arc::new(StdMutex::new(None));
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_http(
        tx,
        200,
        r#"{"choices":[{"message":{"content":[{"type":"text","text":"数组内容"}]}}]}"#,
        captured,
    ));
    let addr = rx.await.unwrap();

    let provider = MultimodalAsrProvider;
    let session = provider
        .connect(&rest_cfg(
            ProviderKind::MultimodalAsr,
            &format!("http://{addr}"),
        ))
        .await
        .expect("connect");
    let finals = run_session(session).await;
    assert_eq!(finals, vec!["数组内容".to_string()]);
}

#[tokio::test]
async fn multimodal_asr_http_error_returns_err() {
    let captured: Captured = Arc::new(StdMutex::new(None));
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_http(tx, 403, r#"{"error":"denied"}"#, captured));
    let addr = rx.await.unwrap();

    let provider = MultimodalAsrProvider;
    let mut session = provider
        .connect(&rest_cfg(
            ProviderKind::MultimodalAsr,
            &format!("http://{addr}"),
        ))
        .await
        .expect("connect");
    session.feed(&test_frame()).await.unwrap();
    let err = session.finish().await;
    assert!(err.is_err(), "HTTP 403 应返回 Err，got {err:?}");
}
