use dioxus::prelude::*;
use queensgame_shared::DISPLAY_NAME_MAX_CHARS;
use queensgame_shared_queens::{Puzzle, PuzzleNav};

pub fn render_puzzles_page(puzzle_nav: Vec<PuzzleNav>, total: usize) -> String {
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
                    a { href: "/minesweeper", "Minesweeper" }
                    a { href: "/rooms", "Rooms" }
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

pub fn render_puzzle_page(puzzle: &Puzzle, bootstrap_json: String) -> String {
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
                    a { href: "/minesweeper", "Minesweeper" }
                    a { href: "/rooms", "Rooms" }
                    a { href: "/puzzles/9x9/{puzzle.id}", "9x9" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "game-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
}

pub fn render_minesweeper_page(bootstrap_json: String) -> String {
    render_document(
        "Minesweeper",
        "Play expert Minesweeper.",
        true,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/minesweeper", "Minesweeper" }
                    a { href: "/rooms", "Rooms" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "minesweeper-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
}

pub fn render_rooms_page() -> String {
    render_document(
        "Queens Game Rooms",
        "Create a multiplayer Queens game room.",
        false,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/minesweeper", "Minesweeper" }
                    a { href: "/rooms", "Rooms" }
                }
            }
            main { class: "archive-page",
                section { class: "archive-hero",
                    p { class: "eyebrow", "Multiplayer" }
                    h1 { "Game Rooms" }
                    p { "Create a room, send the link to other players, ready up, and race the same puzzle together." }
                    form { class: "display-name-form", method: "post", action: "/rooms",
                        label { r#for: "create-display-name", "Display name" }
                        input {
                            id: "create-display-name",
                            name: "display_name",
                            r#type: "text",
                            autocomplete: "nickname",
                            maxlength: "{DISPLAY_NAME_MAX_CHARS}",
                            required: true,
                            placeholder: "Your name"
                        }
                        button { r#type: "submit", class: "nav-button primary", "Create Room" }
                    }
                }
            }
        },
    )
}

pub fn render_room_page(slug: &str, bootstrap_json: String) -> String {
    let title = format!("Queens Room {slug}");

    render_document(
        &title,
        "Join a multiplayer Queens race room.",
        true,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/minesweeper", "Minesweeper" }
                    a { href: "/rooms", "Rooms" }
                    a { href: "/rooms/{slug}", "Room" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "room-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
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
