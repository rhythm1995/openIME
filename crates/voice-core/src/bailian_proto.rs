//! 阿里云百炼 Protocol A 协议帧（流式 ASR）。
//!
//! 对齐 alibabacloud-bailian-speech-demo（默认模型 `fun-asr-realtime`）。
//! 协议要点（见 crate 文档/计划）：
//! - 请求判别字段：`header.action` ∈ {`run-task`, `finish-task`}
//! - 响应判别字段：`header.event` ∈ {`task-started`, `result-generated`,
//!   `task-finished`, `task-failed`}
//! - 音频：WebSocket 二进制帧，LE-i16 / mono / 16kHz
//! - `sentence.text` 是单句语义，不跨句累计
//!
//! 本模块只做 (de)serialize，不含网络逻辑，便于纯单测。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

// ──────────────────────── 请求 ────────────────────────

/// 客户端→服务端的动作判别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    RunTask,
    FinishTask,
}

/// `run-task` / `finish-task` 的公共信封。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub header: RequestHeader,
    pub payload: RequestPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestHeader {
    pub action: Action,
    pub task_id: String,
    pub streaming: String, // 固定 "duplex"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPayload {
    #[serde(default)]
    pub task_group: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 固定为 {}；finish-task 也需要。
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Parameters>,
}

/// `run-task` 的参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameters {
    #[serde(rename = "format")]
    pub format: String,
    #[serde(rename = "sample_rate")]
    pub sample_rate: u32,
    #[serde(
        rename = "language_hints",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub language_hints: Option<Vec<String>>,
    #[serde(rename = "punctuation_prediction_enabled", default)]
    pub punctuation_prediction_enabled: bool,
    #[serde(rename = "inverse_text_normalization_enabled", default)]
    pub inverse_text_normalization_enabled: bool,
    #[serde(rename = "semantic_punctuation_enabled", default)]
    pub semantic_punctuation_enabled: bool,
    #[serde(
        rename = "max_sentence_silence",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_sentence_silence: Option<u32>,
    #[serde(
        rename = "vocabulary_id",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub vocabulary_id: Option<String>,
}

/// 构造 run-task 请求的便捷函数。
pub fn run_task(task_id: &str, model: &str, params: Parameters) -> RequestEnvelope {
    RequestEnvelope {
        header: RequestHeader {
            action: Action::RunTask,
            task_id: task_id.to_string(),
            streaming: "duplex".into(),
        },
        payload: RequestPayload {
            task_group: Some("audio".into()),
            task: Some("asr".into()),
            function: Some("recognition".into()),
            model: Some(model.to_string()),
            input: Value::Object(serde_json::Map::new()),
            parameters: Some(params),
        },
    }
}

/// 构造 finish-task 请求。
pub fn finish_task(task_id: &str) -> RequestEnvelope {
    RequestEnvelope {
        header: RequestHeader {
            action: Action::FinishTask,
            task_id: task_id.to_string(),
            streaming: "duplex".into(),
        },
        payload: RequestPayload {
            task_group: None,
            task: None,
            function: None,
            model: None,
            input: Value::Object(serde_json::Map::new()),
            parameters: None,
        },
    }
}

/// 默认参数：PCM 16k，开标点与 ITN，VAD 切句。
pub fn default_params() -> Parameters {
    Parameters {
        format: "pcm".into(),
        sample_rate: 16_000,
        language_hints: Some(vec!["zh".into(), "en".into()]),
        punctuation_prediction_enabled: true,
        inverse_text_normalization_enabled: true,
        semantic_punctuation_enabled: false,
        max_sentence_silence: Some(800),
        vocabulary_id: None,
    }
}

// ──────────────────────── 响应 ────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Event {
    TaskStarted,
    ResultGenerated,
    TaskFinished,
    TaskFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub header: ResponseHeader,
    #[serde(default)]
    pub payload: ResponsePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseHeader {
    pub task_id: String,
    pub event: Event,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResponsePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Output {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentence: Option<Sentence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sentence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub begin_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sentence_end: bool,
    #[serde(default)]
    pub heartbeat: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub begin_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punctuation: Option<String>,
}

/// 把一行 JSON 解析为响应信封。
pub fn parse_response(json: &str) -> crate::Result<ResponseEnvelope> {
    serde_json::from_str(json).map_err(|e| Error::Protocol(format!("响应解析失败: {e}")))
}

/// 把请求信封序列化为 JSON 字符串。
pub fn to_json(req: &RequestEnvelope) -> crate::Result<String> {
    serde_json::to_string(req).map_err(|e| Error::Protocol(format!("请求序列化失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TASK_ID: &str = "2bf83b9a-baeb-4fda-8d9a-000000000000";

    #[test]
    fn run_task_serializes_to_bailian_shape() {
        let req = run_task(
            TASK_ID,
            "fun-asr-realtime",
            Parameters {
                format: "pcm".into(),
                sample_rate: 16_000,
                language_hints: Some(vec!["en".into(), "zh".into()]),
                punctuation_prediction_enabled: true,
                inverse_text_normalization_enabled: true,
                semantic_punctuation_enabled: false,
                max_sentence_silence: Some(1300),
                vocabulary_id: None,
            },
        );
        let json = to_json(&req).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(v["header"]["action"], json!("run-task"));
        assert_eq!(v["header"]["task_id"], json!(TASK_ID));
        assert_eq!(v["header"]["streaming"], json!("duplex"));
        assert_eq!(v["payload"]["task_group"], json!("audio"));
        assert_eq!(v["payload"]["task"], json!("asr"));
        assert_eq!(v["payload"]["function"], json!("recognition"));
        assert_eq!(v["payload"]["model"], json!("fun-asr-realtime"));
        assert_eq!(v["payload"]["input"], json!({}));
        assert_eq!(v["payload"]["parameters"]["format"], json!("pcm"));
        assert_eq!(v["payload"]["parameters"]["sample_rate"], json!(16000));
        assert_eq!(
            v["payload"]["parameters"]["language_hints"],
            json!(["en", "zh"])
        );
        assert_eq!(
            v["payload"]["parameters"]["punctuation_prediction_enabled"],
            json!(true)
        );
        assert_eq!(
            v["payload"]["parameters"]["inverse_text_normalization_enabled"],
            json!(true)
        );
        assert_eq!(
            v["payload"]["parameters"]["semantic_punctuation_enabled"],
            json!(false)
        );
        assert_eq!(
            v["payload"]["parameters"]["max_sentence_silence"],
            json!(1300)
        );
    }

    #[test]
    fn finish_task_has_no_parameters() {
        let req = finish_task(TASK_ID);
        let json = to_json(&req).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["header"]["action"], json!("finish-task"));
        assert_eq!(v["payload"]["input"], json!({}));
        assert!(v["payload"].get("parameters").is_none() || v["payload"]["parameters"].is_null());
        assert!(v["payload"].get("model").is_none() || v["payload"]["model"].is_null());
    }

    #[test]
    fn parses_task_started() {
        let json = r#"{
            "header": {"task_id":"x","event":"task-started","attributes":{}},
            "payload": {}
        }"#;
        let r = parse_response(json).unwrap();
        assert_eq!(r.header.event, Event::TaskStarted);
        assert_eq!(r.header.task_id, "x");
    }

    #[test]
    fn parses_partial_result() {
        let json = r#"{
            "header": {"task_id":"x","event":"result-generated","attributes":{}},
            "payload": {"output": {"sentence": {
                "begin_time": 170, "end_time": null,
                "text": "好，我知道了",
                "heartbeat": false, "sentence_end": false,
                "words": [{"begin_time":170,"end_time":295,"text":"好","punctuation":"，"}]
            }}, "usage": null}
        }"#;
        let r = parse_response(json).unwrap();
        assert_eq!(r.header.event, Event::ResultGenerated);
        let s = r.payload.output.unwrap().sentence.unwrap();
        assert!(!s.sentence_end);
        assert_eq!(s.text, "好，我知道了");
        assert_eq!(s.words.len(), 1);
        assert_eq!(s.words[0].punctuation.as_deref(), Some("，"));
    }

    #[test]
    fn parses_final_result() {
        let json = r#"{
            "header": {"task_id":"x","event":"result-generated","attributes":{}},
            "payload": {"output": {"sentence": {
                "begin_time":170,"end_time":920,"text":"好，我知道了",
                "heartbeat":false,"sentence_end":true,"words":[]
            }}, "usage": {"duration":3}}
        }"#;
        let r = parse_response(json).unwrap();
        let s = r.payload.output.unwrap().sentence.unwrap();
        assert!(s.sentence_end);
        assert_eq!(s.end_time, Some(920));
        assert!(r.payload.usage.is_some());
    }

    #[test]
    fn parses_task_failed() {
        let json = r#"{
            "header": {"task_id":"x","event":"task-failed",
                        "error_code":"CLIENT_ERROR",
                        "error_message":"request timeout after 23 seconds.",
                        "attributes":{}},
            "payload": {}
        }"#;
        let r = parse_response(json).unwrap();
        assert_eq!(r.header.event, Event::TaskFailed);
        assert_eq!(r.header.error_code.as_deref(), Some("CLIENT_ERROR"));
        assert_eq!(
            r.header.error_message.as_deref(),
            Some("request timeout after 23 seconds.")
        );
    }

    #[test]
    fn parses_task_finished() {
        let json = r#"{
            "header": {"task_id":"x","event":"task-finished","attributes":{}},
            "payload": {"output":{},"usage":null}
        }"#;
        let r = parse_response(json).unwrap();
        assert_eq!(r.header.event, Event::TaskFinished);
    }

    #[test]
    fn roundtrip_request_via_json() {
        let req = run_task(TASK_ID, "paraformer-realtime-v2", default_params());
        let json = to_json(&req).unwrap();
        // 重新反序列化为 Value 验证可读
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["payload"]["model"], "paraformer-realtime-v2");
    }
}
