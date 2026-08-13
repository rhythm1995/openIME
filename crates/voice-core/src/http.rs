//! R3:禁止跟随重定向的 HTTP 客户端（请求期 SSRF 防御）。

use std::time::Duration;

/// 构造禁止跟随重定向的 reqwest 客户端。
///
/// R3 策略：保存期字面校验 + 请求期禁止 redirect。所有用户可配置的 HTTP 客户端
/// （polish/cloud、openai_asr、multimodal_asr、test_connection）都应走它，
/// 避免 3xx 把带 API key 的请求重定向到攻击者 / 内网。
pub fn http_client_no_redirect(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client 构建不应失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_without_redirect() {
        // 仅验证可构建且不 panic；不触网。
        let _c = http_client_no_redirect(Duration::from_secs(5));
    }
}
