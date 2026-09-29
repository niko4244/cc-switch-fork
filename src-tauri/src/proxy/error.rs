use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// 路由失败的结构化元数据（§6.4）。
///
/// `code` 是稳定错误码（见 [`super::log_codes::fo`]），与
/// `active_route::last_error_code` 以及前端 `RouteStatusStrip` 使用同一套取值；
/// `message` 是客户端可见的简短原因；`remedy` 是可直接照做的下一步。
///
/// 这些字段附着在 [`ProxyError::NoProvidersConfigured`] /
/// [`ProxyError::AllProvidersCircuitOpen`] 变体上，因此任何拿到该错误的调用方
/// （HTTP 响应、使用量日志、前端）都能直接取用，无需再各自硬编码一句文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingError {
    pub code: &'static str,
    pub message: &'static str,
    pub remedy: &'static str,
}

impl RoutingError {
    /// `FO-005`：未选中供应商，且故障转移队列为空，无从路由。
    pub const fn no_providers_configured() -> Self {
        Self {
            code: super::log_codes::fo::NO_PROVIDERS,
            message: "No provider is selected and the failover queue is empty, so the request could not be routed.",
            remedy: "Enable or log in to a provider in the provider list, or add one to the failover queue, then retry.",
        }
    }

    /// `FO-004`：候选供应商全部处于熔断冷却中。
    pub const fn all_providers_circuit_open() -> Self {
        Self {
            code: super::log_codes::fo::ALL_CIRCUIT_OPEN,
            message: "Every candidate provider for this app is circuit-broken and cooling down, so the request could not be routed.",
            remedy: "Reset the breaker or re-enable a provider, or wait for the cooldown to elapse, then retry.",
        }
    }
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("服务器已在运行")]
    AlreadyRunning,

    #[error("服务器未运行")]
    NotRunning,

    #[error("地址绑定失败: {0}")]
    BindFailed(String),

    #[error("停止超时")]
    StopTimeout,

    #[error("停止失败: {0}")]
    StopFailed(String),

    #[error("请求转发失败: {0}")]
    ForwardFailed(String),

    #[error("无可用的Provider")]
    NoAvailableProvider,

    #[error("{0}")]
    AllProvidersCircuitOpen(RoutingError),

    #[error("{0}")]
    NoProvidersConfigured(RoutingError),

    #[allow(dead_code)]
    #[error("Provider不健康: {0}")]
    ProviderUnhealthy(String),

    #[error("上游错误 (状态码 {status}): {body:?}")]
    UpstreamError { status: u16, body: Option<String> },

    #[error("超过最大重试次数")]
    MaxRetriesExceeded,

    #[error("数据库错误: {0}")]
    DatabaseError(String),

    #[error("配置错误: {0}")]
    ConfigError(String),

    #[allow(dead_code)]
    #[error("格式转换错误: {0}")]
    TransformError(String),

    #[allow(dead_code)]
    #[error("无效的请求: {0}")]
    InvalidRequest(String),

    #[error("超时: {0}")]
    Timeout(String),

    /// 流式响应空闲超时
    #[allow(dead_code)]
    #[error("流式响应空闲超时: {0}秒无数据")]
    StreamIdleTimeout(u64),

    /// 认证错误
    #[error("认证失败: {0}")]
    AuthError(String),

    #[allow(dead_code)]
    #[error("内部错误: {0}")]
    Internal(String),
}

impl ProxyError {
    /// 构造 `FO-005` 路由错误（未选中供应商且故障转移队列为空）。
    pub fn no_providers_configured() -> Self {
        Self::NoProvidersConfigured(RoutingError::no_providers_configured())
    }

    /// 构造 `FO-004` 路由错误（候选供应商全部处于熔断冷却）。
    pub fn all_providers_circuit_open() -> Self {
        Self::AllProvidersCircuitOpen(RoutingError::all_providers_circuit_open())
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ProxyError::UpstreamError {
                status: upstream_status,
                body: upstream_body,
            } => {
                let http_status =
                    StatusCode::from_u16(*upstream_status).unwrap_or(StatusCode::BAD_GATEWAY);

                // 尝试解析上游响应体为 JSON，如果失败则包装为字符串
                let error_body = if let Some(body_str) = upstream_body {
                    if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                        // 上游返回的是 JSON，直接透传
                        json_body
                    } else {
                        // 上游返回的不是 JSON，包装为错误消息
                        json!({
                            "error": {
                                "message": body_str,
                                "type": "upstream_error",
                            }
                        })
                    }
                } else {
                    json!({
                        "error": {
                            "message": format!("Upstream error (status {})", upstream_status),
                            "type": "upstream_error",
                        }
                    })
                };

                (http_status, error_body)
            }
            _ => {
                let (http_status, message) = match &self {
                    ProxyError::AlreadyRunning => (StatusCode::CONFLICT, self.to_string()),
                    ProxyError::NotRunning => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
                    ProxyError::BindFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopTimeout => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ForwardFailed(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
                    ProxyError::NoAvailableProvider => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::AllProvidersCircuitOpen(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::NoProvidersConfigured(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::ProviderUnhealthy(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::MaxRetriesExceeded => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::DatabaseError(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ConfigError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::TransformError(_) => {
                        (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
                    }
                    ProxyError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
                    ProxyError::StreamIdleTimeout(_) => {
                        (StatusCode::GATEWAY_TIMEOUT, self.to_string())
                    }
                    ProxyError::AuthError(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
                    ProxyError::Internal(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::UpstreamError { .. } => unreachable!(),
                };

                // 路由错误额外带上稳定错误码与可操作建议（§6.4），让客户端
                // 不必去解析本地化文案就能分支处理。
                let error_body = match &self {
                    ProxyError::NoProvidersConfigured(info)
                    | ProxyError::AllProvidersCircuitOpen(info) => json!({
                        "error": {
                            "message": message,
                            "type": "proxy_error",
                            "code": info.code,
                            "remedy": info.remedy,
                        }
                    }),
                    _ => json!({
                        "error": {
                            "message": message,
                            "type": "proxy_error",
                        }
                    }),
                };

                (http_status, error_body)
            }
        };

        (status, Json(body)).into_response()
    }
}

/// 错误分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// 可重试错误（网络问题、5xx）
    Retryable, // 网络超时、5xx 错误
    /// 不可重试错误（4xx、认证失败）
    NonRetryable, // 认证失败、参数错误、4xx 错误
    #[allow(dead_code)]
    ClientAbort, // 客户端主动中断
}

/// 判断错误是否可重试
#[allow(dead_code)]
pub fn categorize_error(error: &reqwest::Error) -> ErrorCategory {
    if error.is_timeout() || error.is_connect() {
        return ErrorCategory::Retryable;
    }

    if let Some(status) = error.status() {
        if status.is_server_error() {
            ErrorCategory::Retryable
        } else if status.is_client_error() {
            ErrorCategory::NonRetryable
        } else {
            ErrorCategory::Retryable
        }
    } else {
        ErrorCategory::Retryable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    /// §6.4：客户端应从错误体里直接拿到稳定错误码与可操作建议，而不是去解析文案。
    #[tokio::test]
    async fn routing_errors_expose_code_and_remedy_in_the_json_body() {
        for (error, code) in [
            (ProxyError::no_providers_configured(), "FO-005"),
            (ProxyError::all_providers_circuit_open(), "FO-004"),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

            let bytes = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("error body");
            let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");

            assert_eq!(body["error"]["code"], code);
            assert_eq!(body["error"]["type"], "proxy_error");
            assert!(body["error"]["remedy"]
                .as_str()
                .is_some_and(|remedy| !remedy.trim().is_empty()));
            assert!(body["error"]["message"]
                .as_str()
                .is_some_and(|message| !message.trim().is_empty()));
        }
    }

    /// 非路由错误不应多出 `code` / `remedy` 字段，免得前端误以为可分支。
    #[tokio::test]
    async fn non_routing_errors_keep_the_plain_body() {
        let response = ProxyError::NoAvailableProvider.into_response();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");

        assert!(body["error"].get("code").is_none());
        assert!(body["error"].get("remedy").is_none());
    }

    #[test]
    fn routing_error_metadata_matches_the_log_code_constants() {
        assert_eq!(
            RoutingError::no_providers_configured().code,
            crate::proxy::log_codes::fo::NO_PROVIDERS
        );
        assert_eq!(
            RoutingError::all_providers_circuit_open().code,
            crate::proxy::log_codes::fo::ALL_CIRCUIT_OPEN
        );
    }
}
