use queensgame_shared_minesweeper::MinesweeperBootstrap;
use queensgame_shared_queens::{GameBootstrap, PuzzleArchiveBootstrap};
use queensgame_shared_room::RoomBootstrap;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum AppBootstrap {
    Puzzles(PuzzleArchiveBootstrap),
    Game(GameBootstrap),
    Minesweeper(MinesweeperBootstrap),
    Rooms,
    Room(RoomBootstrap),
}

#[derive(Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum AppRoute {
    Puzzles(PuzzleArchiveBootstrap),
    Game(GameBootstrap),
    Minesweeper(MinesweeperBootstrap),
    Rooms,
    Room(RoomBootstrap),
    Error(String),
}

impl From<AppBootstrap> for AppRoute {
    fn from(bootstrap: AppBootstrap) -> Self {
        match bootstrap {
            AppBootstrap::Puzzles(bootstrap) => Self::Puzzles(bootstrap),
            AppBootstrap::Game(bootstrap) => Self::Game(bootstrap),
            AppBootstrap::Minesweeper(bootstrap) => Self::Minesweeper(bootstrap),
            AppBootstrap::Rooms => Self::Rooms,
            AppBootstrap::Room(bootstrap) => Self::Room(bootstrap),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PuzzleHistoryAction {
    Push,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    None,
    Push,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteRequest {
    Puzzles,
    Game(usize),
    Minesweeper,
    Rooms,
    Room(String),
}

#[must_use]
pub fn route_request_from_path(path: &str) -> Option<RouteRequest> {
    let path = path
        .split_once(['?', '#'])
        .map_or(path, |(path, _)| path)
        .trim_end_matches('/');
    match path {
        "" | "/" => Some(RouteRequest::Game(1)),
        "/puzzles" | "/puzzles/9x9" => Some(RouteRequest::Puzzles),
        "/minesweeper" => Some(RouteRequest::Minesweeper),
        "/rooms" => Some(RouteRequest::Rooms),
        path if path.starts_with("/puzzles/9x9/") => path
            .trim_start_matches("/puzzles/9x9/")
            .parse()
            .ok()
            .map(RouteRequest::Game),
        path if path.starts_with("/rooms/") => {
            let slug = path.trim_start_matches("/rooms/");
            (!slug.is_empty() && !slug.contains('/')).then(|| RouteRequest::Room(slug.to_string()))
        }
        _ => None,
    }
}

#[must_use]
pub fn app_path_from_href(href: &str) -> Option<String> {
    let path = href.strip_prefix(window_origin().as_str()).unwrap_or(href);
    if !path.starts_with('/') {
        return None;
    }
    match route_request_from_path(path) {
        Some(RouteRequest::Room(_)) | None => None,
        Some(_) => Some(path.to_string()),
    }
}

#[must_use]
pub fn puzzle_page_path(puzzle_id: usize) -> String {
    format!("/puzzles/9x9/{puzzle_id}")
}

#[allow(clippy::missing_const_for_fn)]
fn window_origin() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.location().origin().ok())
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_parser_accepts_app_paths_only() {
        assert_eq!(
            route_request_from_path("/puzzles/9x9/42"),
            Some(RouteRequest::Game(42))
        );
        assert_eq!(
            route_request_from_path("/puzzles/9x9"),
            Some(RouteRequest::Puzzles)
        );
        assert_eq!(
            route_request_from_path("/minesweeper"),
            Some(RouteRequest::Minesweeper)
        );
        assert_eq!(route_request_from_path("/rooms"), Some(RouteRequest::Rooms));
        assert_eq!(
            route_request_from_path("/rooms/ROOMTEST"),
            Some(RouteRequest::Room("ROOMTEST".to_string()))
        );
        assert_eq!(
            app_path_from_href("/puzzles/9x9/7#archive"),
            Some("/puzzles/9x9/7#archive".to_string())
        );
        assert_eq!(
            app_path_from_href("/puzzles/9x9/7?from=header"),
            Some("/puzzles/9x9/7?from=header".to_string())
        );
        assert_eq!(app_path_from_href("/rooms/ROOMTEST"), None);
        assert_eq!(route_request_from_path("/puzzles/8x8/42"), None);
        assert_eq!(route_request_from_path("/puzzles/9x9/nope"), None);
    }
}
