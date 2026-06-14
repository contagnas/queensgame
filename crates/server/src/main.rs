use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use dioxus::prelude::*;
use queensgame_shared::{
    validate_solution, GameBootstrap, Puzzle, PuzzleFile, PuzzleNav, ValidateRequest,
    ValidateResponse,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{services::ServeDir, trace::TraceLayer};

const PUZZLE_DATA: &str = include_str!("../../../data/9x9-puzzles.json");
const STYLE_CSS: &str = include_str!("../../../static/style.css");
const QUEEN_SVG: &str = include_str!("../../../static/queen.svg");

#[derive(Clone)]
struct AppState {
    puzzles: Arc<Vec<Puzzle>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "queensgame=info,tower_http=info".into()),
        )
        .init();

    let state = AppState {
        puzzles: Arc::new(load_puzzles()),
    };
    let client_dist = client_dist_dir();

    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/puzzles/9x9/1") }))
        .route("/puzzles", get(puzzles_index))
        .route("/puzzles/9x9", get(puzzles_index))
        .route("/puzzles/9x9/:id", get(puzzle_page))
        .route("/api/puzzles/9x9/:id", get(puzzle_api))
        .route("/api/validate", post(validate_api))
        .route("/static/style.css", get(static_css))
        .route("/static/queen.svg", get(static_queen_svg))
        .nest_service("/static/client", ServeDir::new(client_dist))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = "127.0.0.1:3000"
        .parse()
        .expect("hard-coded listen address is valid");
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HTTP listener");
    axum::serve(listener, app)
        .await
        .expect("HTTP server failed");
}

fn client_dist_dir() -> PathBuf {
    std::env::var_os("QUEENSGAME_CLIENT_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist/client"))
}

fn load_puzzles() -> Vec<Puzzle> {
    let data: PuzzleFile = serde_json::from_str(PUZZLE_DATA)
        .expect("data/9x9-puzzles.json must contain valid puzzle data");
    assert!(!data.puzzles.is_empty(), "puzzle data must not be empty");
    data.puzzles
}

async fn puzzles_index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    Ok(Html(render_puzzles_page(
        puzzle_nav(&state.puzzles, 0),
        state.puzzles.len(),
    )))
}

async fn puzzle_page(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Html<String>, AppError> {
    let puzzle = find_puzzle(&state, id)?.clone();
    let bootstrap = GameBootstrap {
        puzzle: puzzle.clone(),
        puzzle_nav: puzzle_nav(&state.puzzles, id),
        total: state.puzzles.len(),
    };
    let bootstrap_json = serde_json::to_string(&bootstrap)?;

    Ok(Html(render_puzzle_page(&puzzle, bootstrap_json)))
}

async fn puzzle_api(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Json<Puzzle>, AppError> {
    Ok(Json(find_puzzle(&state, id)?.clone()))
}

async fn validate_api(
    State(state): State<AppState>,
    Json(request): Json<ValidateRequest>,
) -> Result<Json<ValidateResponse>, AppError> {
    let puzzle = find_puzzle(&state, request.id)?;
    Ok(Json(validate_solution(puzzle, &request.queens)))
}

async fn static_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

async fn static_queen_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        QUEEN_SVG,
    )
}

fn find_puzzle(state: &AppState, id: usize) -> Result<&Puzzle, AppError> {
    state
        .puzzles
        .iter()
        .find(|puzzle| puzzle.id == id)
        .ok_or(AppError::NotFound)
}

fn puzzle_nav(puzzles: &[Puzzle], active_id: usize) -> Vec<PuzzleNav> {
    puzzles
        .iter()
        .map(|puzzle| PuzzleNav {
            id: puzzle.id,
            active: puzzle.id == active_id,
        })
        .collect()
}

fn render_document(title: &str, description: &str, client: bool, content: Element) -> String {
    let content = dioxus_ssr::render_element(content);
    let client_script = if client {
        r#"<script type="module">import init from "/static/client/queensgame_client.js"; init();</script>"#
    } else {
        ""
    };

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><meta name="description" content="{description}"><link rel="stylesheet" href="/static/style.css"></head><body>{content}{client_script}</body></html>"#
    )
}

fn render_puzzles_page(puzzle_nav: Vec<PuzzleNav>, total: usize) -> String {
    render_document(
        "9x9 Queens Puzzles",
        "Choose from 300 bundled 9x9 Queens puzzles.",
        false,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                }
            }
            main { class: "archive-page",
                section { class: "archive-hero",
                    p { class: "eyebrow", "Puzzle set" }
                    h1 { "9x9 Queens Puzzles" }
                    p { "{total} advanced boards with one queen per row, column, and colored region." }
                    a { class: "nav-button primary", href: "/puzzles/9x9/1", "Start Puzzle #1" }
                }
                section { class: "archive-list", aria_labelledby: "archive-title",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Archive" }
                        h2 { id: "archive-title", "Select a puzzle" }
                    }
                    div { class: "puzzle-grid wide",
                        for nav in puzzle_nav {
                            a { href: "/puzzles/9x9/{nav.id}", "{nav.id}" }
                        }
                    }
                }
            }
        },
    )
}

fn render_puzzle_page(puzzle: &Puzzle, bootstrap_json: String) -> String {
    let title = format!("9x9 Queens Puzzle #{}", puzzle.id);

    render_document(
        &title,
        "Place one queen in every row, column, and colored region without diagonal touching.",
        true,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/puzzles/9x9/{puzzle.id}", "9x9" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "game-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
}

#[derive(Debug)]
enum AppError {
    NotFound,
    Json(serde_json::Error),
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        AppError::Json(error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Puzzle not found").into_response(),
            AppError::Json(error) => {
                tracing::error!(%error, "JSON handling failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "JSON handling failed").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puzzle_data_contains_9x9_puzzles() {
        let puzzles = load_puzzles();
        assert_eq!(puzzles.len(), 300);
        for puzzle in puzzles {
            assert_eq!(puzzle.size, 9);
            assert_eq!(puzzle.colors.len(), 9);
            assert_eq!(puzzle.regions.len(), 9);
            assert!(puzzle.regions.iter().all(|row| row.len() == 9));
        }
    }
}
