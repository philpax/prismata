use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{DefaultBodyLimit, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use prismata_protocol::{self as protocol, tools::prism};
use tower_http::cors::CorsLayer;

pub mod comfyui;

mod prism_render;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    pub http_port: Option<u16>,
    pub logger: bool,
    pub comfyui_address: String,
    /// Passed as `Bearer {comfyui_authentication}` in the `Authorization` header
    /// to the ComfyUI server (i.e. <https://github.com/ai-dock/comfyui>)
    pub comfyui_authentication: Option<String>,
    /// If true, the server will accept self-signed certificates from the ComfyUI server
    pub comfyui_https_insecure: bool,
    pub workflow: comfyui::WorkflowSettings,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            http_port: None,
            logger: true,
            comfyui_address: "http://127.0.0.1:8188".to_string(),
            comfyui_authentication: None,
            comfyui_https_insecure: false,
            workflow: Default::default(),
        }
    }
}

pub async fn main(settings: &Settings) {
    if settings.logger {
        tracing_subscriber::fmt::init();
    }

    let reqwest_client = reqwest::Client::builder();
    let reqwest_client = if settings.comfyui_https_insecure {
        reqwest_client.danger_accept_invalid_certs(true)
    } else {
        reqwest_client
    };
    let reqwest_client = if let Some(authentication) = &settings.comfyui_authentication {
        reqwest_client.default_headers(
            std::iter::once((
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", authentication))
                    .expect("Failed to create header value"),
            ))
            .collect(),
        )
    } else {
        reqwest_client
    };
    let reqwest_client = reqwest_client
        .build()
        .expect("Failed to build reqwest client");

    let rucomfyui_client = Arc::new(rucomfyui::Client::new_with_client(
        &settings.comfyui_address,
        reqwest_client,
    ));
    let app = Router::new()
        .route(
            "/version",
            get(|| async {
                format!(
                    "v{}-{}",
                    env!("CARGO_PKG_VERSION"),
                    git_version::git_version!(fallback = "unknown")
                )
            }),
        )
        .route("/prism/render", post(post_prism_render))
        .layer(DefaultBodyLimit::disable())
        .layer(CorsLayer::very_permissive())
        .with_state(AppState {
            rucomfyui_client,
            workflow_settings: settings.workflow.clone(),
        });

    let addr = SocketAddr::from((
        [0, 0, 0, 0],
        settings
            .http_port
            .unwrap_or(protocol::DEFAULT_HTTP_SERVER_PORT),
    ));

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to address");
    tracing::info!("HTTP server listening on {addr}");
    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}

#[derive(Clone)]
struct AppState {
    rucomfyui_client: Arc<rucomfyui::Client>,
    workflow_settings: comfyui::WorkflowSettings,
}

async fn post_prism_render(
    State(state): State<AppState>,
    Json(request): Json<prism::RenderInput>,
) -> Json<prism::RenderOutputResult> {
    let result =
        prism_render::queue_prompt(&state.rucomfyui_client, &state.workflow_settings, &request)
            .await;

    match result {
        Ok(output) => Json(prism::RenderOutputResult::Ok(output)),
        Err(err) => Json(prism::RenderOutputResult::Err(err.to_string())),
    }
}
