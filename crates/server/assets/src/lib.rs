use axum::{http::header, response::IntoResponse};
use queensgame_shared_queens::{Puzzle, PuzzleFile};

const PUZZLE_DATA: &str = include_str!("../../../../data/9x9-puzzles.json");
const STYLE_CSS: &str = include_str!("../../../../static/style.css");
const MAGE_SVG: &str = include_str!("../../../../static/mage.svg");
const MAGE_GREY: &str = "#d0d2d6";
const MAGE_PURPLE: &str = "#8445c3";
const MAGE_SWAP_SENTINEL: &str = "__BOARDMAGE_MAGE_SWAP__";
const QUEEN_SVG: &str = include_str!("../../../../static/queen.svg");
const MINESWEEPER_FLAG_SVG: &str = include_str!("../../../../static/minesweeper-flag.svg");
const MINESWEEPER_MINE_SVG: &str = include_str!("../../../../static/minesweeper-mine.svg");
const DSEG7_CLASSIC_BOLD_WOFF2: &[u8] =
    include_bytes!("../../../../static/fonts/dseg7-classic-bold.woff2");

/// Loads the embedded puzzle catalog.
///
/// # Panics
///
/// Panics if the checked-in puzzle data is invalid or empty.
#[must_use]
pub fn load_puzzles() -> Vec<Puzzle> {
    let data: PuzzleFile = serde_json::from_str(PUZZLE_DATA)
        .expect("data/9x9-puzzles.json must contain valid puzzle data");
    assert!(!data.puzzles.is_empty(), "puzzle data must not be empty");
    data.puzzles
}

#[must_use]
pub fn static_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

#[must_use]
pub fn static_mage_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        MAGE_SVG,
    )
}

#[must_use]
pub fn static_mage_light_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        MAGE_SVG
            .replace(MAGE_GREY, MAGE_SWAP_SENTINEL)
            .replace(MAGE_PURPLE, MAGE_GREY)
            .replace(MAGE_SWAP_SENTINEL, MAGE_PURPLE),
    )
}

#[must_use]
pub fn static_queen_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        QUEEN_SVG,
    )
}

#[must_use]
pub fn static_minesweeper_flag_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        MINESWEEPER_FLAG_SVG,
    )
}

#[must_use]
pub fn static_minesweeper_mine_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        MINESWEEPER_MINE_SVG,
    )
}

#[must_use]
pub fn static_dseg7_classic_bold_woff2() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "font/woff2")],
        DSEG7_CLASSIC_BOLD_WOFF2,
    )
}
