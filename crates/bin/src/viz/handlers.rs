//! Axum handlers for the DAG visualizer.
//!
//! All routes here piggy-back on the existing dashboard's session-cookie auth
//! and scope every database open through the user's tracked-database API,
//! so a user can only visualize databases they already have access to.
//!
//! Routes (mounted by `commands::serve`):
//!
//! - `GET /dashboard/database/visualize?id=<root_id>` — HTML page hosting the
//!   single-page visualizer.
//! - `GET /static/viz.css` and `GET /static/viz.js` — embedded frontend assets.
//! - `GET /api/dashboard/database/:id/graph` — JSON nodes/edges for the DAG.
//! - `GET /api/dashboard/database/:id/entries/:eid` — full detail for a single entry.
//! - `GET /api/dashboard/database/:id/subtrees` — JSON list of subtree names.
//!
//! The JSON wire formats are defined in [`super::graph`] and intentionally
//! tagged with a `kind` discriminator on nodes and edges so future link types
//! (IPLD object refs, cross-database pointers, …) are additive.

// axum response bodies are large structs by design; the helper functions in
// this module return `Result<T, Response>` so each handler can short-circuit
// with a typed error response. The clippy lint is not actionable here.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use eidetica::{Database, ID, user::User};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_cookies::Cookies;

use super::{assets, graph as g};
use crate::commands::serve::{AppState, SESSION_COOKIE};

/// Query parameters for the visualizer page.
#[derive(Deserialize)]
pub struct VisualizeQuery {
    pub id: String,
}

/// Generic JSON error envelope for the API endpoints.
#[derive(Serialize)]
struct ApiError {
    error: String,
}

impl ApiError {
    fn into_response(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}

fn parse_id(s: &str) -> Result<ID, Response> {
    ID::parse(s).map_err(|_| {
        ApiError {
            error: format!("Invalid ID: {s}"),
        }
        .into_response(StatusCode::BAD_REQUEST)
    })
}

/// Look up the active session and return the wrapped User, or redirect-to-login.
async fn require_user(state: &AppState, cookies: &Cookies) -> Result<Arc<RwLock<User>>, Response> {
    let token = cookies
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| Redirect::to("/login").into_response())?;
    state
        .sessions
        .get_user(&token)
        .await
        .ok_or_else(|| Redirect::to("/login").into_response())
}

/// API equivalent of [`require_user`] that returns a JSON 401 instead of a redirect.
async fn require_user_api(
    state: &AppState,
    cookies: &Cookies,
) -> Result<Arc<RwLock<User>>, Response> {
    let token = cookies
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| {
            ApiError {
                error: "Not signed in".into(),
            }
            .into_response(StatusCode::UNAUTHORIZED)
        })?;
    state.sessions.get_user(&token).await.ok_or_else(|| {
        ApiError {
            error: "Session expired".into(),
        }
        .into_response(StatusCode::UNAUTHORIZED)
    })
}

/// Open a database the user has access to, or return a JSON error response.
async fn open_db(user: &User, id: &ID) -> Result<Database, Response> {
    user.open_database(id).await.map_err(|e| {
        let status = if e.is_not_found() {
            StatusCode::NOT_FOUND
        } else if e.is_permission_denied() || e.is_authentication_error() {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError {
            error: format!("Failed to open database: {e}"),
        }
        .into_response(status)
    })
}

/// `GET /dashboard/database/visualize?id=<root_id>`
pub async fn handle_visualize_page(
    State(state): State<AppState>,
    cookies: Cookies,
    Query(query): Query<VisualizeQuery>,
) -> Response {
    let user_lock = match require_user(&state, &cookies).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let database_id = match ID::parse(&query.id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid database ID").into_response(),
    };

    let user = user_lock.read().await;
    let db = match user.open_database(&database_id).await {
        Ok(d) => d,
        Err(e) => {
            let status = if e.is_not_found() {
                StatusCode::NOT_FOUND
            } else if e.is_permission_denied() || e.is_authentication_error() {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (status, format!("Failed to open database: {e}")).into_response();
        }
    };

    let name = db.get_name().await.unwrap_or_else(|_| query.id.clone());
    let html = assets::render_html(&query.id, &name);
    Html(html).into_response()
}

/// `GET /static/viz.css`
pub async fn handle_viz_css() -> impl IntoResponse {
    static_response(assets::VIZ_CSS, "text/css; charset=utf-8")
}

/// `GET /static/viz.js`
pub async fn handle_viz_js() -> impl IntoResponse {
    static_response(assets::VIZ_JS, "application/javascript; charset=utf-8")
}

fn static_response(body: &'static str, content_type: &'static str) -> Response {
    let mut resp = body.into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // Light caching: the assets are embedded in the binary, so they don't
    // change at runtime. Cache for an hour; long enough to feel snappy but
    // short enough that a binary upgrade is picked up promptly.
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    resp
}

/// `GET /api/dashboard/database/:id/graph`
pub async fn handle_api_graph(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(id): Path<String>,
) -> Response {
    let user_lock = match require_user_api(&state, &cookies).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let database_id = match parse_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let user = user_lock.read().await;
    let db = match open_db(&user, &database_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    match g::build_graph(&db).await {
        Ok(graph) => Json(graph).into_response(),
        Err(e) => ApiError {
            error: format!("Failed to build graph: {e}"),
        }
        .into_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `GET /api/dashboard/database/:id/entries/:eid`
pub async fn handle_api_entry(
    State(state): State<AppState>,
    cookies: Cookies,
    Path((id, eid)): Path<(String, String)>,
) -> Response {
    let user_lock = match require_user_api(&state, &cookies).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let database_id = match parse_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let entry_id = match parse_id(&eid) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let user = user_lock.read().await;
    let db = match open_db(&user, &database_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    match g::build_entry_detail(&db, &entry_id).await {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => {
            let status = if e.is_not_found() {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ApiError {
                error: format!("Failed to read entry: {e}"),
            }
            .into_response(status)
        }
    }
}

/// `GET /api/dashboard/database/:id/subtrees`
pub async fn handle_api_subtrees(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(id): Path<String>,
) -> Response {
    let user_lock = match require_user_api(&state, &cookies).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let database_id = match parse_id(&id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let user = user_lock.read().await;
    let db = match open_db(&user, &database_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    match g::build_summary(&db).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => ApiError {
            error: format!("Failed to summarize database: {e}"),
        }
        .into_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
