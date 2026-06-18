use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use queensgame_server_assets::{
    static_css, static_dseg7_classic_bold_woff2, static_mage_light_svg, static_mage_svg,
    static_minesweeper_flag_svg, static_minesweeper_mine_svg, static_queen_svg,
};
use queensgame_server_pages::render_app_page;
use queensgame_server_rooms::{
    AppState, create_room_api, create_room_form, room_api, room_page, room_ws, rooms_index,
};
use queensgame_server_runtime::{bind_addr, client_dist_dir};
use queensgame_shared_minesweeper::MinesweeperBootstrap;
use queensgame_shared_nonogram::NonogramBootstrap;
use queensgame_shared_queens::{GameBootstrap, PuzzleArchiveBootstrap, load_puzzles};
use tower_http::{services::ServeDir, trace::TraceLayer};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "queensgame=info,tower_http=info".into()),
        )
        .init();

    let state = AppState::new(load_puzzles());
    let client_dist = client_dist_dir();

    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/puzzles/9x9/1") }))
        .route("/puzzles", get(puzzles_index))
        .route("/puzzles/9x9", get(puzzles_index))
        .route("/puzzles/9x9/:id", get(puzzle_page))
        .route("/minesweeper", get(minesweeper_page))
        .route("/nonograms", get(nonogram_page))
        .route("/rooms", get(rooms_index).post(create_room_form))
        .route("/rooms/:slug", get(room_page))
        .route("/api/rooms", post(create_room_api))
        .route("/api/rooms/:slug", get(room_api))
        .route("/ws/rooms/:slug", get(room_ws))
        .route("/favicon.svg", get(|| async { static_mage_svg() }))
        .route(
            "/static/mage-light.svg",
            get(|| async { static_mage_light_svg() }),
        )
        .route("/static/mage.svg", get(|| async { static_mage_svg() }))
        .route("/static/style.css", get(|| async { static_css() }))
        .route("/static/queen.svg", get(|| async { static_queen_svg() }))
        .route(
            "/static/minesweeper-flag.svg",
            get(|| async { static_minesweeper_flag_svg() }),
        )
        .route(
            "/static/minesweeper-mine.svg",
            get(|| async { static_minesweeper_mine_svg() }),
        )
        .route(
            "/static/fonts/dseg7-classic-bold.woff2",
            get(|| async { static_dseg7_classic_bold_woff2() }),
        )
        .nest_service("/static/client", ServeDir::new(client_dist))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = bind_addr();
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HTTP listener");
    axum::serve(listener, app)
        .await
        .expect("HTTP server failed");
}

async fn puzzles_index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let app_json = app_json("puzzles", &puzzle_archive_bootstrap(&state));
    Ok(Html(render_app_page(
        "Boardmage - 9x9 Queens Puzzles",
        "Choose from 300 bundled 9x9 Queens boards.",
        &app_json,
    )))
}

async fn puzzle_page(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Html<String>, AppError> {
    let bootstrap = puzzle_bootstrap(&state, id)?;
    let app_json = app_json("game", &bootstrap);

    Ok(Html(render_app_page(
        &format!("Boardmage - Queens Puzzle #{}", bootstrap.puzzle.id),
        "Place one queen in every row, column, and colored region without diagonal touching.",
        &app_json,
    )))
}

async fn minesweeper_page() -> Result<Html<String>, AppError> {
    let app_json = app_json("minesweeper", &MinesweeperBootstrap::default());

    Ok(Html(render_app_page(
        "Boardmage - Minesweeper",
        "Play expert Minesweeper.",
        &app_json,
    )))
}

async fn nonogram_page() -> Result<Html<String>, AppError> {
    let app_json = app_json("nonogram", &NonogramBootstrap::default());

    Ok(Html(render_app_page(
        "Boardmage - Nonograms",
        "Solve generated picture logic puzzles.",
        &app_json,
    )))
}

fn puzzle_bootstrap(state: &AppState, id: usize) -> Result<GameBootstrap, AppError> {
    queensgame_shared_queens::puzzle_bootstrap(&state.puzzles, id).ok_or(AppError::NotFound)
}

fn puzzle_archive_bootstrap(state: &AppState) -> PuzzleArchiveBootstrap {
    queensgame_shared_queens::puzzle_archive_bootstrap(&state.puzzles)
}

fn app_json<T: serde::Serialize>(kind: &str, data: &T) -> String {
    serde_json::json!({
        "kind": kind,
        "data": data,
    })
    .to_string()
}

#[derive(Debug)]
enum AppError {
    NotFound,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "Not found").into_response(),
        }
    }
}
