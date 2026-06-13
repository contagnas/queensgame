use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;

const PUZZLE_DATA: &str = include_str!("../data/9x9-puzzles.json");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");
const QUEEN_SVG: &str = include_str!("../static/queen.svg");

#[derive(Clone)]
struct AppState {
    puzzles: Arc<Vec<Puzzle>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PuzzleFile {
    puzzles: Vec<Puzzle>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Puzzle {
    id: usize,
    size: usize,
    colors: Vec<String>,
    regions: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, Serialize)]
struct CellView {
    row: usize,
    col: usize,
    region: usize,
    color: String,
    border_top: bool,
    border_right: bool,
    border_bottom: bool,
    border_left: bool,
}

impl CellView {
    fn class_name(&self) -> String {
        let mut class_name = String::from("cell");
        if self.border_top {
            class_name.push_str(" border-top");
        }
        if self.border_right {
            class_name.push_str(" border-right");
        }
        if self.border_bottom {
            class_name.push_str(" border-bottom");
        }
        if self.border_left {
            class_name.push_str(" border-left");
        }
        class_name
    }
}

#[derive(Debug, Clone)]
struct PuzzleNav {
    id: usize,
    active: bool,
}

#[derive(Debug, Deserialize)]
struct ValidateRequest {
    id: usize,
    queens: Vec<[usize; 2]>,
}

#[derive(Debug, Serialize)]
struct ValidateResponse {
    complete: bool,
    queen_count: usize,
    expected_queens: usize,
    satisfied_rows: usize,
    satisfied_columns: usize,
    satisfied_regions: usize,
    conflict_cells: Vec<[usize; 2]>,
    messages: Vec<String>,
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

    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/puzzles/9x9/1") }))
        .route("/puzzles", get(puzzles_index))
        .route("/puzzles/9x9", get(puzzles_index))
        .route("/puzzles/9x9/:id", get(puzzle_page))
        .route("/api/puzzles/9x9/:id", get(puzzle_api))
        .route("/api/validate", post(validate_api))
        .route("/static/app.js", get(static_js))
        .route("/static/style.css", get(static_css))
        .route("/static/queen.svg", get(static_queen_svg))
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
    let total = state.puzzles.len();
    let puzzle_json = serde_json::to_string(&puzzle)?;

    Ok(Html(render_puzzle_page(
        puzzle.clone(),
        build_cells(&puzzle),
        puzzle_json,
        puzzle_nav(&state.puzzles, id),
        total,
    )))
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

async fn static_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
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

fn build_cells(puzzle: &Puzzle) -> Vec<CellView> {
    let size = puzzle.size;
    let mut cells = Vec::with_capacity(size * size);

    for row in 0..size {
        for col in 0..size {
            let region = puzzle.regions[row][col];
            cells.push(CellView {
                row,
                col,
                region,
                color: puzzle.colors[region].clone(),
                border_top: row == 0 || puzzle.regions[row - 1][col] != region,
                border_right: col + 1 == size,
                border_bottom: row + 1 == size,
                border_left: col == 0 || puzzle.regions[row][col - 1] != region,
            });
        }
    }

    cells
}

fn render_document(title: &str, description: &str, content: Element) -> String {
    let content = dioxus_ssr::render_element(content);
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><meta name="description" content="{description}"><link rel="stylesheet" href="/static/style.css"><script defer src="/static/app.js"></script></head><body>{content}</body></html>"#
    )
}

fn render_puzzles_page(puzzle_nav: Vec<PuzzleNav>, total: usize) -> String {
    render_document(
        "9x9 Queens Puzzles",
        "Choose from 300 bundled 9x9 Queens puzzles.",
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
                    p { class: "eyebrow", "bundled starter set" }
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

fn render_puzzle_page(
    puzzle: Puzzle,
    cells: Vec<CellView>,
    puzzle_json: String,
    puzzle_nav: Vec<PuzzleNav>,
    total: usize,
) -> String {
    let has_prev = puzzle.id > 1;
    let prev_id = puzzle.id.saturating_sub(1);
    let has_next = puzzle.id < total;
    let next_id = puzzle.id + 1;
    let title = format!("9x9 Queens Puzzle #{}", puzzle.id);

    render_document(
        &title,
        "Place one queen in every row, column, and colored region without diagonal touching.",
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
            main { class: "game-page",
                section { class: "game-shell", aria_labelledby: "game-title",
                    div { class: "game-toolbar",
                        div {
                            p { class: "eyebrow", "9x9 puzzle {puzzle.id} of {total}" }
                            h1 { id: "game-title", "Queens Puzzle #{puzzle.id}" }
                        }
                        div { class: "timer-box", aria_live: "polite",
                            span { class: "timer-label", "Time" }
                            span { id: "timer", "00:00" }
                        }
                    }
                    div { class: "controls-row", aria_label: "Game controls",
                        div { class: "segmented", role: "group", aria_label: "Cell mode",
                            button { r#type: "button", class: "mode-button active", "data-mode": "queen", "Queen" }
                            button { r#type: "button", class: "mode-button", "data-mode": "mark", "Mark" }
                            button { r#type: "button", class: "mode-button", "data-mode": "clear", "Clear" }
                        }
                        div { class: "tool-buttons",
                            button { r#type: "button", class: "tool-button", id: "undo-button", title: "Undo last move", "Undo" }
                            button { r#type: "button", class: "tool-button", id: "hint-button", title: "Highlight conflicts", "Check" }
                            button { r#type: "button", class: "tool-button", id: "reset-button", title: "Reset this puzzle", "Reset" }
                        }
                    }
                    div { class: "board-wrap",
                        div { class: "board", role: "grid", aria_label: "Queens board", style: "--board-size: {puzzle.size}",
                            for cell in cells {
                                button {
                                    r#type: "button",
                                    class: "{cell.class_name()}",
                                    style: "--cell-color: {cell.color}",
                                    "data-row": "{cell.row}",
                                    "data-col": "{cell.col}",
                                    "data-region": "{cell.region}",
                                    aria_label: "Row {cell.row + 1}, column {cell.col + 1}",
                                    role: "gridcell",
                                    span { class: "cell-symbol", aria_hidden: "true" }
                                }
                            }
                        }
                    }
                    div { class: "status-strip", aria_live: "polite",
                        span { id: "queen-count", "0 / {puzzle.size} queens" }
                        span { id: "rule-status", "Rows, columns, and regions update as you play." }
                    }
                    div { class: "rule-panel",
                        h2 { "Rules" }
                        p { "Place exactly one queen in each row, column, and colored region. Queens may not touch diagonally." }
                    }
                    div { class: "puzzle-actions",
                        if has_prev {
                            a { class: "nav-button", href: "/puzzles/9x9/{prev_id}", "Previous" }
                        }
                        if has_next {
                            a { class: "nav-button primary", href: "/puzzles/9x9/{next_id}", "Next Puzzle" }
                        }
                    }
                }
                aside { class: "side-panel", aria_label: "Puzzle selector",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Archive" }
                        h2 { "9x9 puzzles" }
                    }
                    div { class: "puzzle-grid",
                        for nav in puzzle_nav {
                            a {
                                class: if nav.active { "active" } else { "" },
                                href: "/puzzles/9x9/{nav.id}",
                                "{nav.id}"
                            }
                        }
                    }
                }
            }
            div { class: "win-dialog", id: "win-dialog", hidden: true,
                div { class: "win-panel", role: "dialog", aria_modal: "true", aria_labelledby: "win-title",
                    p { class: "eyebrow", "Solved" }
                    h2 { id: "win-title", "Puzzle complete" }
                    p { id: "win-time", "Finished in 00:00." }
                    div { class: "dialog-actions",
                        button { r#type: "button", class: "tool-button", id: "close-win", "Keep Playing" }
                        if has_next {
                            a { class: "nav-button primary", href: "/puzzles/9x9/{next_id}", "Next Puzzle" }
                        }
                    }
                }
            }
            script { r#type: "application/json", id: "puzzle-data", dangerous_inner_html: "{puzzle_json}" }
        },
    )
}

fn validate_solution(puzzle: &Puzzle, queens: &[[usize; 2]]) -> ValidateResponse {
    let size = puzzle.size;
    let mut row_counts = vec![0usize; size];
    let mut col_counts = vec![0usize; size];
    let mut region_counts = vec![0usize; size];
    let mut conflict_cells = BTreeSet::new();
    let mut messages = Vec::new();
    let mut valid_queens = Vec::new();

    for &[row, col] in queens {
        if row >= size || col >= size {
            messages.push(format!(
                "Ignored out-of-bounds queen at {},{}.",
                row + 1,
                col + 1
            ));
            continue;
        }

        valid_queens.push([row, col]);
        row_counts[row] += 1;
        col_counts[col] += 1;
        region_counts[puzzle.regions[row][col]] += 1;
    }

    for queen in &valid_queens {
        let [row, col] = *queen;
        let region = puzzle.regions[row][col];

        if row_counts[row] > 1 || col_counts[col] > 1 || region_counts[region] > 1 {
            conflict_cells.insert([row, col]);
        }
    }

    for i in 0..valid_queens.len() {
        for j in (i + 1)..valid_queens.len() {
            let [row_a, col_a] = valid_queens[i];
            let [row_b, col_b] = valid_queens[j];
            let row_delta = row_a.abs_diff(row_b);
            let col_delta = col_a.abs_diff(col_b);
            if row_delta == 1 && col_delta == 1 {
                conflict_cells.insert([row_a, col_a]);
                conflict_cells.insert([row_b, col_b]);
            }
        }
    }

    let satisfied_rows = row_counts.iter().filter(|&&count| count == 1).count();
    let satisfied_columns = col_counts.iter().filter(|&&count| count == 1).count();
    let satisfied_regions = region_counts.iter().filter(|&&count| count == 1).count();

    if valid_queens.len() != size {
        messages.push(format!("Place exactly {size} queens."));
    }
    if satisfied_rows != size {
        messages.push("Each row needs one queen.".to_string());
    }
    if satisfied_columns != size {
        messages.push("Each column needs one queen.".to_string());
    }
    if satisfied_regions != size {
        messages.push("Each colored region needs one queen.".to_string());
    }
    if !conflict_cells.is_empty() {
        messages
            .push("Queens cannot share a row, column, region, or touch diagonally.".to_string());
    }

    let complete = valid_queens.len() == size
        && satisfied_rows == size
        && satisfied_columns == size
        && satisfied_regions == size
        && conflict_cells.is_empty();

    ValidateResponse {
        complete,
        queen_count: valid_queens.len(),
        expected_queens: size,
        satisfied_rows,
        satisfied_columns,
        satisfied_regions,
        conflict_cells: conflict_cells.into_iter().collect(),
        messages,
    }
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
    fn bundled_data_contains_9x9_puzzles() {
        let puzzles = load_puzzles();
        assert_eq!(puzzles.len(), 300);
        for puzzle in puzzles {
            assert_eq!(puzzle.size, 9);
            assert_eq!(puzzle.colors.len(), 9);
            assert_eq!(puzzle.regions.len(), 9);
            assert!(puzzle.regions.iter().all(|row| row.len() == 9));
        }
    }

    #[test]
    fn validation_rejects_diagonal_touching() {
        let puzzle = Puzzle {
            id: 1,
            size: 2,
            colors: vec!["#000000".into(), "#FFFFFF".into()],
            regions: vec![vec![0, 0], vec![1, 1]],
        };

        let response = validate_solution(&puzzle, &[[0, 0], [1, 1]]);
        assert!(!response.complete);
        assert_eq!(response.conflict_cells.len(), 2);
    }
}
