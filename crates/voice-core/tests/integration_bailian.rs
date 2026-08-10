//! 百炼 provider 集成测试：起本地 WS mock server 模拟百炼 Protocol A，
//! 验证 BailianProvider 全流程：握手鉴权 → run-task → task-started →
//! 收音频 → result-generated(partial+final) → finish-task → task-finished。

use std::net::SocketAddr;

use futures::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use voice_core::providers::bailian::BailianProvider;
use voice_core::traits::{AsrProvider, AudioFormat, AudioFrame};
use voice_core::{ProviderConfig, ProviderKind};

/// mock server 先绑定，把真实地址通过 tx 返回，再开始接受连接。
async fn serve_full_flow(tx: oneshot::Sender<SocketAddr>) {
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tx.send(addr).unwrap();

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

    // run-task
    let run_task_msg = ws.next().await.unwrap().unwrap().into_text().unwrap();
    assert!(
        run_task_msg.contains("\"action\":\"run-task\""),
        "got: {run_task_msg}"
    );
    assert!(run_task_msg.contains("fun-asr-realtime"));

    // task-started
    ws.send(Message::Text(
        r#"{"header":{"task_id":"t","event":"task-started","attributes":{}},"payload":{}}"#.into(),
    ))
    .await
    .unwrap();

    // 吃 3 帧音频
    for _ in 0..3 {
        let _ = ws.next().await.unwrap().unwrap();
    }

    // partial + final
    ws.send(Message::Text(
        r#"{"header":{"task_id":"t","event":"result-generated","attributes":{}},
            "payload":{"output":{"sentence":{"begin_time":0,"end_time":null,
            "text":"你好","heartbeat":false,"sentence_end":false,"words":[]}},"usage":null}}"#
            .into(),
    ))
    .await
    .unwrap();
    ws.send(Message::Text(
        r#"{"header":{"task_id":"t","event":"result-generated","attributes":{}},
            "payload":{"output":{"sentence":{"begin_time":0,"end_time":500,
            "text":"你好世界","heartbeat":false,"sentence_end":true,"words":[]}},"usage":{"duration":1}}}"#
            .into(),
    ))
    .await
    .unwrap();

    // 等 finish-task，回 task-finished
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) if t.contains("\"action\":\"finish-task\"") => break,
            _ => {}
        }
    }
    ws.send(Message::Text(
        r#"{"header":{"task_id":"t","event":"task-finished","attributes":{}},
            "payload":{"output":{},"usage":null}}"#
            .into(),
    ))
    .await
    .unwrap();
}

async fn serve_fail(tx: oneshot::Sender<SocketAddr>) {
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tx.send(addr).unwrap();

    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    let _ = ws.next().await.unwrap().unwrap(); // run-task
    ws.send(Message::Text(
        r#"{"header":{"task_id":"t","event":"task-failed",
        "error_code":"CLIENT_ERROR","error_message":"bad model","attributes":{}},
        "payload":{}}"#
            .into(),
    ))
    .await
    .unwrap();
}

fn test_frame() -> AudioFrame {
    AudioFrame::new(AudioFormat::PCM_16K_MONO_S16LE, vec![0u8; 640]) // 20ms
}

#[tokio::test]
async fn bailian_full_flow() {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_full_flow(tx));
    let addr = rx.await.unwrap();

    let cfg = ProviderConfig {
        kind: ProviderKind::Bailian,
        base_url: format!("ws://{addr}"),
        api_key: "sk-test".into(),
        model: "fun-asr-realtime".into(),
        vocabulary_id: None,
    };

    let provider = BailianProvider;
    let mut session = provider.connect(&cfg).await.expect("connect");

    for _ in 0..3 {
        session.feed(&test_frame()).await.unwrap();
    }
    session.finish().await.unwrap();

    let mut deltas = session.deltas();
    let mut got_partial = false;
    let mut got_final = false;
    while let Some(Ok(d)) = deltas.next().await {
        match d.kind {
            voice_core::TranscriptKind::Partial => {
                assert_eq!(d.text, "你好");
                got_partial = true;
            }
            voice_core::TranscriptKind::Final => {
                assert_eq!(d.text, "你好世界");
                got_final = true;
            }
        }
    }
    assert!(got_partial, "应收到 partial");
    assert!(got_final, "应收到 final");
}

#[tokio::test]
async fn bailian_reports_task_failed() {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(serve_fail(tx));
    let addr = rx.await.unwrap();

    let cfg = ProviderConfig {
        kind: ProviderKind::Bailian,
        base_url: format!("ws://{addr}"),
        api_key: "sk-test".into(),
        model: "fun-asr-realtime".into(),
        vocabulary_id: None,
    };

    let provider = BailianProvider;
    let result = provider.connect(&cfg).await;
    let err = match result {
        Ok(_) => panic!("应失败但成功了"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("bad model") || msg.contains("CLIENT_ERROR"),
        "got: {msg}"
    );
}
