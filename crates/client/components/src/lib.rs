use dioxus::prelude::*;
use queensgame_shared_room::RoomPlayerSnapshot;

#[derive(Clone, PartialEq, Eq)]
pub struct RoomPlayerListRow {
    pub player: RoomPlayerSnapshot,
    pub status: String,
}

#[component]
pub fn MinesweeperLed(label: String, value: String) -> Element {
    rsx! {
        div { class: "ms-led", aria_label: "{label}",
            "{value}"
        }
    }
}

#[component]
pub fn RoomPlayersPanel(rows: Vec<RoomPlayerListRow>, winner_id: Option<String>) -> Element {
    rsx! {
        aside { class: "side-panel", aria_label: "Players",
            div { class: "selector-header",
                p { class: "eyebrow", "Players" }
                h2 { "{rows.len()} in room" }
            }
            div { class: "player-list",
                for row in rows.iter() {
                    div {
                        class: player_row_class(&row.player, winner_id.as_deref()),
                        span { class: "player-name", "{row.player.name}" }
                        span { class: "player-status", "{row.status}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn RoomLeaderboardPanel(players: Vec<RoomPlayerSnapshot>) -> Element {
    let leaderboard_players = sorted_leaderboard_players(&players);
    let leaderboard_max_total = leaderboard_players
        .iter()
        .map(|player| player.medals.total())
        .max()
        .unwrap_or(0)
        .max(1);

    rsx! {
        aside { class: "side-panel leaderboard-panel", aria_label: "Leaderboard",
            div { class: "selector-header",
                p { class: "eyebrow", "Leaderboard" }
                h2 { "Medals" }
            }
            div { class: "leaderboard-list",
                for player in leaderboard_players.iter() {
                    {
                        let total = player.medals.total();
                        let gold_width = medal_width(player.medals.gold, leaderboard_max_total);
                        let silver_width = medal_width(player.medals.silver, leaderboard_max_total);
                        let bronze_width = medal_width(player.medals.bronze, leaderboard_max_total);

                        rsx! {
                            div { class: "leaderboard-row",
                                div { class: "leaderboard-row-header",
                                    span { class: "player-name", "{player.name}" }
                                    span { class: "leaderboard-total", "{total}" }
                                }
                                div { class: "medal-counts", aria_label: "Medal counts",
                                    span { class: "medal-count gold", "G {player.medals.gold}" }
                                    span { class: "medal-count silver", "S {player.medals.silver}" }
                                    span { class: "medal-count bronze", "B {player.medals.bronze}" }
                                }
                                div { class: "medal-bars", aria_hidden: "true",
                                    span { class: "medal-bar gold", style: "width: {gold_width}" }
                                    span { class: "medal-bar silver", style: "width: {silver_width}" }
                                    span { class: "medal-bar bronze", style: "width: {bronze_width}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn RoomStatusBox(status: String) -> Element {
    rsx! {
        div { class: "timer-box room-status-box", aria_live: "polite",
            span { class: "timer-label", "Status" }
            span { "{status}" }
        }
    }
}

#[component]
pub fn RoomMinesweeperScores(players: Vec<RoomPlayerSnapshot>) -> Element {
    rsx! {
        section { class: "room-ms-scores", aria_label: "Minesweeper scores",
            div { class: "selector-header",
                p { class: "eyebrow", "Scores" }
                h2 { "Final standings" }
            }
            div { class: "leaderboard-list",
                for (index, player) in players.iter().enumerate() {
                    {
                        let place = ordinal_place(index + 1);
                        let place_label = if place.is_empty() {
                            format!("#{}", index + 1)
                        } else {
                            place.to_string()
                        };

                        rsx! {
                            div { class: "leaderboard-row room-ms-score-row",
                                div { class: "leaderboard-row-header",
                                    span { class: "player-name", "{place_label} {player.name}" }
                                    span { class: "leaderboard-total", "{player.minesweeper_score}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn player_row_class(player: &RoomPlayerSnapshot, winner_id: Option<&str>) -> &'static str {
    if winner_id == Some(player.id.as_str()) {
        "player-row winner"
    } else if !player.connected {
        "player-row disconnected"
    } else {
        "player-row"
    }
}

fn sorted_leaderboard_players(players: &[RoomPlayerSnapshot]) -> Vec<RoomPlayerSnapshot> {
    let mut players = players.to_vec();
    players.sort_by(|left, right| {
        right
            .medals
            .gold
            .cmp(&left.medals.gold)
            .then_with(|| right.medals.silver.cmp(&left.medals.silver))
            .then_with(|| right.medals.bronze.cmp(&left.medals.bronze))
            .then_with(|| left.name.cmp(&right.name))
    });
    players
}

fn medal_width(count: u32, max_total: u32) -> String {
    if count == 0 {
        "0%".to_string()
    } else {
        format!("{:.2}%", count as f64 / max_total.max(1) as f64 * 100.0)
    }
}

fn ordinal_place(place: usize) -> &'static str {
    match place {
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        _ => "",
    }
}
