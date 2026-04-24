/// OAuth 2.0 Authorization Server — implementación mínima para satisfacer
/// el spec MCP 2025-03-26 y permitir que claude.ai web conecte servidores
/// MCP remotos sin credenciales propietarias.
///
/// Flujo implementado: Authorization Code + PKCE (S256)
/// El "usuario" es el token MCP (ej: "hvera"), que se ingresa en la página /authorize.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Form, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

// ─── Estado compartido ────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct OAuthState {
    inner: Arc<OAuthInner>,
}

struct OAuthInner {
    base_url: String,
    /// code → (user_token, code_challenge, expiry)
    codes: Mutex<HashMap<String, PendingCode>>,
    /// client_id → redirect_uris registradas
    clients: Mutex<HashMap<String, Vec<String>>>,
}

struct PendingCode {
    user_token: String,
    code_challenge: Option<String>,
    expires_at: Instant,
}

impl OAuthState {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(OAuthInner {
                base_url: base_url.into(),
                codes: Mutex::new(HashMap::new()),
                clients: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    fn issue_code(&self, user_token: String, code_challenge: Option<String>) -> String {
        let code = Uuid::new_v4().to_string();
        self.inner.codes.lock().unwrap().insert(code.clone(), PendingCode {
            user_token,
            code_challenge,
            expires_at: Instant::now() + Duration::from_secs(300),
        });
        code
    }

    fn exchange_code(&self, code: &str, code_verifier: Option<&str>) -> Option<String> {
        let mut codes = self.inner.codes.lock().unwrap();
        let pending = codes.remove(code)?;
        if pending.expires_at < Instant::now() {
            return None;
        }
        // Verificar PKCE si fue provisto
        if let Some(challenge) = &pending.code_challenge {
            let verifier = code_verifier?;
            let hash = Sha256::digest(verifier.as_bytes());
            let computed = URL_SAFE_NO_PAD.encode(hash);
            if &computed != challenge {
                return None;
            }
        }
        Some(pending.user_token)
    }

    fn register_client(&self, redirect_uris: Vec<String>) -> String {
        let client_id = Uuid::new_v4().to_string();
        self.inner.clients.lock().unwrap().insert(client_id.clone(), redirect_uris);
        client_id
    }
}

// ─── Router público ───────────────────────────────────────────────────────────

pub fn router(state: OAuthState) -> Router {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(protected_resource))
        .route("/.well-known/oauth-protected-resource/mcp", get(protected_resource))
        .route("/.well-known/oauth-authorization-server", get(authorization_server))
        .route("/register", post(register))
        .route("/authorize", get(authorize_get).post(authorize_post))
        .route("/token", post(token))
        .with_state(state)
}

// ─── Discovery endpoints ──────────────────────────────────────────────────────

async fn protected_resource(State(s): State<OAuthState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "resource": s.base_url(),
        "authorization_servers": [s.base_url()]
    }))
}

async fn authorization_server(State(s): State<OAuthState>) -> impl IntoResponse {
    let base = s.base_url();
    Json(serde_json::json!({
        "issuer": base,
        "authorization_endpoint": format!("{}/authorize", base),
        "token_endpoint": format!("{}/token", base),
        "registration_endpoint": format!("{}/register", base),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"]
    }))
}

// ─── Registro dinámico de clientes (RFC 7591) ─────────────────────────────────

#[derive(Deserialize)]
struct RegisterRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

#[derive(Serialize)]
struct RegisterResponse {
    client_id: String,
    redirect_uris: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<String>,
}

async fn register(
    State(s): State<OAuthState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let client_id = s.register_client(req.redirect_uris.clone());
    Json(RegisterResponse {
        client_id,
        redirect_uris: req.redirect_uris,
        client_name: req.client_name,
    })
}

// ─── Autorización ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AuthorizeQuery {
    redirect_uri: String,
    state: Option<String>,
    code_challenge: Option<String>,
    #[serde(default)]
    code_challenge_method: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    response_type: Option<String>,
}

async fn authorize_get(Query(params): Query<AuthorizeQuery>) -> impl IntoResponse {
    let html = format!(r#"<!DOCTYPE html>
<html lang="es">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>TtimeBot — Autorizar acceso</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 400px; margin: 80px auto; padding: 0 20px; color: #1a1a1a; }}
  h1 {{ font-size: 1.4rem; margin-bottom: 0.25rem; }}
  p {{ color: #666; font-size: 0.9rem; margin-bottom: 1.5rem; }}
  label {{ display: block; font-size: 0.85rem; font-weight: 600; margin-bottom: 0.4rem; }}
  input {{ width: 100%; padding: 0.6rem 0.8rem; border: 1px solid #ccc; border-radius: 6px; font-size: 1rem; box-sizing: border-box; }}
  button {{ margin-top: 1rem; width: 100%; padding: 0.7rem; background: #2563eb; color: white; border: none; border-radius: 6px; font-size: 1rem; cursor: pointer; }}
  button:hover {{ background: #1d4ed8; }}
</style>
</head>
<body>
<h1>🕐 TtimeBot</h1>
<p>Ingresá tu token para autorizar el acceso a TrackingTime.</p>
<form method="POST">
  <input type="hidden" name="redirect_uri" value="{redirect_uri}">
  <input type="hidden" name="state" value="{state}">
  <input type="hidden" name="code_challenge" value="{code_challenge}">
  <label for="token">Tu token</label>
  <input type="text" id="token" name="token" placeholder="ej: hvera" required autofocus>
  <button type="submit">Autorizar</button>
</form>
</body>
</html>"#,
        redirect_uri = html_escape(&params.redirect_uri),
        state = html_escape(&params.state.clone().unwrap_or_default()),
        code_challenge = html_escape(&params.code_challenge.clone().unwrap_or_default()),
    );
    Html(html)
}

#[derive(Deserialize)]
struct AuthorizeForm {
    token: String,
    redirect_uri: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code_challenge: Option<String>,
}

async fn authorize_post(
    State(s): State<OAuthState>,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let code = s.issue_code(
        form.token.trim().to_string(),
        form.code_challenge.filter(|c| !c.is_empty()),
    );

    let mut url = form.redirect_uri.clone();
    url.push_str(if url.contains('?') { "&" } else { "?" });
    url.push_str(&format!("code={}", code));
    if let Some(state) = &form.state {
        if !state.is_empty() {
            url.push_str(&format!("&state={}", state));
        }
    }

    Redirect::to(&url).into_response()
}

// ─── Token endpoint ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    code_verifier: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
}

async fn token(
    State(s): State<OAuthState>,
    Form(req): Form<TokenRequest>,
) -> Response {
    if req.grant_type != "authorization_code" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "unsupported_grant_type"
        }))).into_response();
    }

    let code = match &req.code {
        Some(c) => c.as_str(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "invalid_request",
            "error_description": "missing code"
        }))).into_response(),
    };

    let user_token = match s.exchange_code(code, req.code_verifier.as_deref()) {
        Some(t) => t,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "code inválido o expirado"
        }))).into_response(),
    };

    (StatusCode::OK, Json(serde_json::json!({
        "access_token": user_token,
        "token_type": "Bearer",
        "expires_in": 2592000
    }))).into_response()
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ─── Extracción de token desde Bearer header ──────────────────────────────────

/// Extrae el user token desde `Authorization: Bearer <token>` o `?token=` query param.
pub fn extract_token(headers: &axum::http::HeaderMap, query_token: &str) -> String {
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(val) = auth.to_str() {
            if let Some(bearer) = val.strip_prefix("Bearer ") {
                return bearer.trim().to_string();
            }
        }
    }
    query_token.to_string()
}
