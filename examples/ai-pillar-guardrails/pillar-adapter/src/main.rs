//! Pillar Security Webhook Adapter for AgentGateway
//!
//! This adapter translates between AgentGateway's webhook guardrail format
//! and Pillar Security's API format.
//!
//! Usage:
//!     PILLAR_API_KEY=your-api-key cargo run --release
//!
//! The adapter listens on port 8080 and handles:
//!     - POST /request  - Scan prompts before sending to LLM
//!     - POST /response - Scan LLM responses before returning to client

use axum::{
    Router,
    routing::post,
    extract::State,
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    Json,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{env, sync::Arc, time::Duration};
use tracing::{info, error, warn};

const DEFAULT_PORT: u16 = 8080;
const DEFAULT_PILLAR_URL: &str = "https://api.pillar.security/api/v1";

// ============================================================================
// AgentGateway Webhook Types (input/output)
// ============================================================================

#[derive(Debug, Deserialize)]
struct WebhookRequest {
    body: WebhookBody,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WebhookBody {
    Prompt(PromptMessages),
    Response(ResponseChoices),
}

#[derive(Debug, Deserialize)]
struct PromptMessages {
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct ResponseChoices {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Serialize)]
struct WebhookResponse {
    action: WebhookAction,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum WebhookAction {
    Pass(PassAction),
    Reject(RejectAction),
}

#[derive(Debug, Serialize)]
struct PassAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct RejectAction {
    body: String,
    status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

// ============================================================================
// Pillar API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct PillarScanRequest {
    message: String,
}

/// Response from Pillar scan APIs
/// Each field is a boolean indicating if that type of issue was detected
#[derive(Debug, Deserialize, Default)]
struct PillarScanResponse {
    #[serde(default)]
    jailbreak: bool,
    #[serde(default)]
    prompt_injection: bool,
    #[serde(default)]
    pii: bool,
    #[serde(default)]
    pci: bool,
    #[serde(default)]
    secrets: bool,
    #[serde(default)]
    toxic_language: bool,
    #[serde(default)]
    invisible_characters: bool,
    #[serde(default)]
    evasion: bool,
    #[serde(default)]
    restricted_topics: bool,
    #[serde(default)]
    restricted_keywords: bool,
    #[serde(default)]
    safety: bool,
    #[serde(default)]
    code_detection: bool,
    #[serde(default)]
    findings: Vec<PillarFinding>,
}

#[derive(Debug, Deserialize)]
struct PillarFinding {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

impl PillarScanResponse {
    /// Returns true if any security issue was detected
    fn is_flagged(&self) -> bool {
        self.jailbreak
            || self.prompt_injection
            || self.pii
            || self.pci
            || self.secrets
            || self.toxic_language
            || self.invisible_characters
            || self.evasion
            || self.restricted_topics
            || self.restricted_keywords
            || self.safety
    }
}

// ============================================================================
// Request Context (from forwarded headers)
// ============================================================================

#[derive(Debug, Default)]
struct RequestContext {
    source_ip: Option<String>,
    model: Option<String>,
    service: Option<String>,
    request_id: Option<String>,
    user_id: Option<String>,
}

impl RequestContext {
    fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            source_ip: headers
                .get("x-forwarded-for")
                .or_else(|| headers.get("x-real-ip"))
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(',').next().unwrap_or(s).trim().to_string()),
            model: headers
                .get("x-model")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            service: headers
                .get("x-service")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            request_id: headers
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            user_id: headers
                .get("x-user-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
        }
    }
}

// ============================================================================
// Application State
// ============================================================================

struct AppState {
    client: Client,
    pillar_url: String,
    api_key: String,
}

// ============================================================================
// Handlers
// ============================================================================

async fn handle_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WebhookRequest>,
) -> impl IntoResponse {
    let ctx = RequestContext::from_headers(&headers);

    let messages = match req.body {
        WebhookBody::Prompt(p) => p.messages,
        WebhookBody::Response(_) => {
            warn!("Received response body on /request endpoint");
            return (StatusCode::OK, Json(WebhookResponse {
                action: WebhookAction::Pass(PassAction { reason: Some("invalid request format".into()) }),
            }));
        }
    };

    // Convert messages to prompt string
    let prompt: String = messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    info!(
        source_ip = ctx.source_ip.as_deref().unwrap_or("-"),
        model = ctx.model.as_deref().unwrap_or("-"),
        service = ctx.service.as_deref().unwrap_or("-"),
        user_id = ctx.user_id.as_deref().unwrap_or("-"),
        request_id = ctx.request_id.as_deref().unwrap_or("-"),
        chars = prompt.len(),
        "Scanning prompt"
    );

    match scan_prompt(&state, &prompt).await {
        Ok(pillar_resp) => {
            if pillar_resp.is_flagged() {
                let reason = get_rejection_reason(&pillar_resp);
                info!("Prompt BLOCKED: {}", reason);
                (StatusCode::OK, Json(WebhookResponse {
                    action: WebhookAction::Reject(RejectAction {
                        body: serde_json::json!({
                            "error": {
                                "message": format!("Request blocked by Pillar Security: {}", reason),
                                "type": "content_policy_violation",
                                "code": "guardrail_blocked"
                            }
                        }).to_string(),
                        status_code: 400,
                        reason: Some(reason),
                    }),
                }))
            } else {
                info!("Prompt ALLOWED");
                (StatusCode::OK, Json(WebhookResponse {
                    action: WebhookAction::Pass(PassAction { reason: Some("allowed".into()) }),
                }))
            }
        }
        Err(e) => {
            error!("Pillar API error: {}", e);
            // Fail open - pass through on error
            (StatusCode::OK, Json(WebhookResponse {
                action: WebhookAction::Pass(PassAction { reason: Some(format!("pillar error: {}", e)) }),
            }))
        }
    }
}

async fn handle_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WebhookRequest>,
) -> impl IntoResponse {
    let ctx = RequestContext::from_headers(&headers);

    let choices = match req.body {
        WebhookBody::Response(r) => r.choices,
        WebhookBody::Prompt(_) => {
            warn!("Received prompt body on /response endpoint");
            return (StatusCode::OK, Json(WebhookResponse {
                action: WebhookAction::Pass(PassAction { reason: Some("invalid request format".into()) }),
            }));
        }
    };

    // Convert choices to response string
    let response_text: String = choices
        .iter()
        .map(|c| c.message.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    info!(
        source_ip = ctx.source_ip.as_deref().unwrap_or("-"),
        model = ctx.model.as_deref().unwrap_or("-"),
        service = ctx.service.as_deref().unwrap_or("-"),
        user_id = ctx.user_id.as_deref().unwrap_or("-"),
        request_id = ctx.request_id.as_deref().unwrap_or("-"),
        chars = response_text.len(),
        "Scanning response"
    );

    match scan_response(&state, &response_text).await {
        Ok(pillar_resp) => {
            if pillar_resp.is_flagged() {
                let reason = get_rejection_reason(&pillar_resp);
                info!("Response BLOCKED: {}", reason);
                (StatusCode::OK, Json(WebhookResponse {
                    action: WebhookAction::Reject(RejectAction {
                        body: serde_json::json!({
                            "error": {
                                "message": format!("Response blocked by Pillar Security: {}", reason),
                                "type": "content_policy_violation",
                                "code": "guardrail_blocked"
                            }
                        }).to_string(),
                        status_code: 400,
                        reason: Some(reason),
                    }),
                }))
            } else {
                info!("Response ALLOWED");
                (StatusCode::OK, Json(WebhookResponse {
                    action: WebhookAction::Pass(PassAction { reason: Some("allowed".into()) }),
                }))
            }
        }
        Err(e) => {
            error!("Pillar API error: {}", e);
            // Fail open - pass through on error
            (StatusCode::OK, Json(WebhookResponse {
                action: WebhookAction::Pass(PassAction { reason: Some(format!("pillar error: {}", e)) }),
            }))
        }
    }
}

// ============================================================================
// Pillar API Calls
// ============================================================================

async fn scan_prompt(state: &AppState, message: &str) -> anyhow::Result<PillarScanResponse> {
    let url = format!("{}/scan/prompt", state.pillar_url);
    let resp = state.client
        .post(&url)
        .bearer_auth(&state.api_key)
        .json(&PillarScanRequest { message: message.to_string() })
        .send()
        .await?
        .error_for_status()?
        .json::<PillarScanResponse>()
        .await?;
    Ok(resp)
}

async fn scan_response(state: &AppState, message: &str) -> anyhow::Result<PillarScanResponse> {
    let url = format!("{}/scan/response", state.pillar_url);
    let resp = state.client
        .post(&url)
        .bearer_auth(&state.api_key)
        .json(&PillarScanRequest { message: message.to_string() })
        .send()
        .await?
        .error_for_status()?
        .json::<PillarScanResponse>()
        .await?;
    Ok(resp)
}

fn get_rejection_reason(resp: &PillarScanResponse) -> String {
    let mut reasons = Vec::new();

    if resp.jailbreak { reasons.push("jailbreak attempt"); }
    if resp.prompt_injection { reasons.push("prompt injection"); }
    if resp.pii { reasons.push("PII detected"); }
    if resp.pci { reasons.push("PCI data detected"); }
    if resp.secrets { reasons.push("secret detected"); }
    if resp.toxic_language { reasons.push("toxic language"); }
    if resp.invisible_characters { reasons.push("invisible characters"); }
    if resp.evasion { reasons.push("evasion attempt"); }
    if resp.restricted_topics { reasons.push("restricted topic"); }
    if resp.restricted_keywords { reasons.push("restricted keyword"); }
    if resp.safety { reasons.push("safety concern"); }

    if !reasons.is_empty() {
        return reasons.join(", ");
    }

    if let Some(finding) = resp.findings.first() {
        if let Some(ref cat) = finding.category {
            return cat.clone();
        }
    }

    "content policy violation".to_string()
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("pillar_adapter=info".parse()?)
        )
        .init();

    // Read configuration from environment
    let api_key = env::var("PILLAR_API_KEY")
        .expect("PILLAR_API_KEY environment variable is required");
    let pillar_url = env::var("PILLAR_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_PILLAR_URL.to_string());
    let port: u16 = env::var("ADAPTER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    // Create HTTP client with connection pooling
    let client = Client::builder()
        .user_agent("pillar-adapter/0.1.0")
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30))
        .build()?;

    let state = Arc::new(AppState {
        client,
        pillar_url: pillar_url.clone(),
        api_key,
    });

    // Build router
    let app = Router::new()
        .route("/request", post(handle_request))
        .route("/response", post(handle_response))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("Pillar adapter listening on port {}", port);
    info!("Pillar API URL: {}", pillar_url);

    axum::serve(listener, app).await?;

    Ok(())
}
