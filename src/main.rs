use askama::Template;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
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

#[derive(Debug, Clone)]
struct PuzzleNav {
    id: usize,
    active: bool,
}

#[derive(Template)]
#[template(path = "puzzle.html")]
struct PuzzleTemplate {
    puzzle: Puzzle,
    cells: Vec<CellView>,
    puzzle_json: String,
    puzzle_nav: Vec<PuzzleNav>,
    total: usize,
    has_prev: bool,
    prev_id: usize,
    has_next: bool,
    next_id: usize,
}

#[derive(Template)]
#[template(path = "puzzles.html")]
struct PuzzlesTemplate {
    puzzle_nav: Vec<PuzzleNav>,
    total: usize,
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
    let template = PuzzlesTemplate {
        puzzle_nav: puzzle_nav(&state.puzzles, 0),
        total: state.puzzles.len(),
    };
    render(template)
}

async fn puzzle_page(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Html<String>, AppError> {
    let puzzle = find_puzzle(&state, id)?.clone();
    let total = state.puzzles.len();
    let template = PuzzleTemplate {
        cells: build_cells(&puzzle),
        puzzle_json: serde_json::to_string(&puzzle)?,
        puzzle_nav: puzzle_nav(&state.puzzles, id),
        has_prev: id > 1,
        prev_id: id.saturating_sub(1),
        has_next: id < total,
        next_id: id + 1,
        total,
        puzzle,
    };

    render(template)
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

fn render<T: Template>(template: T) -> Result<Html<String>, AppError> {
    Ok(Html(template.render()?))
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
                border_right: col + 1 == size || puzzle.regions[row][col + 1] != region,
                border_bottom: row + 1 == size || puzzle.regions[row + 1][col] != region,
                border_left: col == 0 || puzzle.regions[row][col - 1] != region,
            });
        }
    }

    cells
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
    Template(askama::Error),
    Json(serde_json::Error),
}

impl From<askama::Error> for AppError {
    fn from(error: askama::Error) -> Self {
        AppError::Template(error)
    }
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
            AppError::Template(error) => {
                tracing::error!(%error, "template rendering failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Template rendering failed",
                )
                    .into_response()
            }
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
