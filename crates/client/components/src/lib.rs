use dioxus::prelude::*;
use queensgame_shared_minesweeper::{MinesweeperCell, MinesweeperCellState, MinesweeperStatus};
use queensgame_shared_room::{RoomMinesweeperCellSnapshot, RoomPlayerSnapshot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinesweeperCellDisplay {
    pub revealed: bool,
    pub mine: bool,
    pub flagged: bool,
    pub question: bool,
    pub pressed: bool,
    pub detonated: bool,
    pub wrong_flag: bool,
    pub adjacent_mines: Option<u8>,
    pub countdown: Option<u8>,
    pub owner_color_index: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinesweeperFaceState {
    Ready,
    Pressed,
    Won,
    Lost,
}

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

pub fn minesweeper_face_symbol(state: MinesweeperFaceState) -> &'static str {
    match state {
        MinesweeperFaceState::Ready => ":)",
        MinesweeperFaceState::Pressed => ":O",
        MinesweeperFaceState::Won => "B)",
        MinesweeperFaceState::Lost => ":(",
    }
}

pub fn minesweeper_cell_text(display: MinesweeperCellDisplay) -> String {
    if let Some(countdown) = display.countdown {
        return countdown.to_string();
    }
    if display.pressed || (display.flagged && !display.revealed) {
        return String::new();
    }
    if display.revealed && !display.mine {
        return display
            .adjacent_mines
            .filter(|adjacent| *adjacent > 0)
            .map(|adjacent| adjacent.to_string())
            .unwrap_or_default();
    }
    String::new()
}

pub fn format_minesweeper_counter(value: i32) -> String {
    let value = value.clamp(-99, 999);
    if value < 0 {
        format!("-{:02}", value.abs())
    } else {
        format!("{value:03}")
    }
}

pub fn minesweeper_cell_class(base_class: &str, display: MinesweeperCellDisplay) -> String {
    let mut class_name = String::from(base_class);
    if display.revealed || display.pressed || display.countdown.is_some() {
        class_name.push_str(" revealed");
    } else {
        class_name.push_str(" raised");
    }
    if display.flagged && !display.revealed {
        class_name.push_str(" flagged");
    }
    if display.question && !display.revealed {
        class_name.push_str(" question");
    }
    if display.revealed && display.mine {
        class_name.push_str(" mine");
    }
    if display.detonated {
        class_name.push_str(" detonated");
    }
    if display.wrong_flag {
        class_name.push_str(" wrong-flag");
    }
    if display.revealed
        && !display.mine
        && let Some(adjacent) = display.adjacent_mines
        && adjacent > 0
    {
        class_name.push_str(&format!(" n{adjacent}"));
    }
    if let Some(countdown) = display.countdown {
        class_name.push_str(&format!(" n{countdown}"));
    }
    if let Some(owner_color_index) = display.owner_color_index {
        class_name.push_str(&format!(" owner-color-{owner_color_index}"));
    }
    class_name
}

pub fn minesweeper_cell_aria(row: usize, col: usize, display: MinesweeperCellDisplay) -> String {
    let state = if let Some(countdown) = display.countdown {
        format!("starting cell {countdown}")
    } else if display.flagged && !display.revealed {
        "flagged".to_string()
    } else if display.question && !display.revealed {
        "question marked".to_string()
    } else if !display.revealed {
        "hidden".to_string()
    } else if display.mine {
        "mine".to_string()
    } else if display.adjacent_mines.unwrap_or_default() == 0 {
        "clear".to_string()
    } else {
        format!(
            "{} adjacent mines",
            display.adjacent_mines.unwrap_or_default()
        )
    };
    format!("Row {}, column {}, {}", row + 1, col + 1, state)
}

pub fn minesweeper_board_cell_display(
    cell: &MinesweeperCell,
    status: MinesweeperStatus,
    pressed: bool,
) -> MinesweeperCellDisplay {
    MinesweeperCellDisplay {
        revealed: cell.state == MinesweeperCellState::Revealed,
        mine: cell.mine,
        flagged: cell.state == MinesweeperCellState::Flagged,
        question: cell.state == MinesweeperCellState::Question,
        pressed,
        detonated: cell.detonated,
        wrong_flag: status == MinesweeperStatus::Lost
            && cell.state == MinesweeperCellState::Flagged
            && !cell.mine,
        adjacent_mines: Some(cell.adjacent_mines),
        countdown: None,
        owner_color_index: None,
    }
}

pub fn room_minesweeper_cell_display(
    cell: &RoomMinesweeperCellSnapshot,
    own_flag: bool,
    pressed: bool,
    start_countdown: Option<u8>,
    owner_color_index: Option<u8>,
) -> MinesweeperCellDisplay {
    MinesweeperCellDisplay {
        revealed: cell.revealed,
        mine: cell.mine,
        flagged: own_flag,
        question: false,
        pressed,
        detonated: cell.detonated,
        wrong_flag: false,
        adjacent_mines: cell.adjacent_mines,
        countdown: start_countdown,
        owner_color_index,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minesweeper_face_symbols_are_shared() {
        assert_eq!(minesweeper_face_symbol(MinesweeperFaceState::Ready), ":)");
        assert_eq!(minesweeper_face_symbol(MinesweeperFaceState::Pressed), ":O");
        assert_eq!(minesweeper_face_symbol(MinesweeperFaceState::Won), "B)");
        assert_eq!(minesweeper_face_symbol(MinesweeperFaceState::Lost), ":(");
    }

    #[test]
    fn minesweeper_counter_formats_to_three_digits() {
        assert_eq!(format_minesweeper_counter(0), "000");
        assert_eq!(format_minesweeper_counter(7), "007");
        assert_eq!(format_minesweeper_counter(1234), "999");
        assert_eq!(format_minesweeper_counter(-1), "-01");
        assert_eq!(format_minesweeper_counter(-123), "-99");
    }

    #[test]
    fn minesweeper_cell_text_hides_unrevealed_marks() {
        assert_eq!(
            minesweeper_cell_text(MinesweeperCellDisplay {
                flagged: true,
                ..MinesweeperCellDisplay::default()
            }),
            ""
        );
    }

    #[test]
    fn minesweeper_cell_text_shows_revealed_numbers_and_countdown() {
        assert_eq!(
            minesweeper_cell_text(MinesweeperCellDisplay {
                revealed: true,
                adjacent_mines: Some(3),
                ..MinesweeperCellDisplay::default()
            }),
            "3"
        );
        assert_eq!(
            minesweeper_cell_text(MinesweeperCellDisplay {
                countdown: Some(5),
                ..MinesweeperCellDisplay::default()
            }),
            "5"
        );
    }

    #[test]
    fn minesweeper_board_cell_display_marks_wrong_flags_after_loss() {
        let display = minesweeper_board_cell_display(
            &MinesweeperCell {
                mine: false,
                adjacent_mines: 0,
                state: MinesweeperCellState::Flagged,
                detonated: false,
            },
            MinesweeperStatus::Lost,
            false,
        );

        assert!(display.flagged);
        assert!(display.wrong_flag);
    }

    #[test]
    fn room_minesweeper_cell_display_maps_countdown_and_owner_color() {
        let display = room_minesweeper_cell_display(
            &RoomMinesweeperCellSnapshot {
                revealed: false,
                mine: false,
                detonated: false,
                start: true,
                adjacent_mines: Some(2),
                owner_id: None,
            },
            true,
            false,
            Some(5),
            Some(3),
        );

        assert!(display.flagged);
        assert_eq!(display.countdown, Some(5));
        assert_eq!(display.owner_color_index, Some(3));
    }

    #[test]
    fn minesweeper_cell_class_adds_common_state_classes() {
        assert_eq!(
            minesweeper_cell_class(
                "ms-cell",
                MinesweeperCellDisplay {
                    revealed: true,
                    adjacent_mines: Some(2),
                    owner_color_index: Some(3),
                    ..MinesweeperCellDisplay::default()
                }
            ),
            "ms-cell revealed n2 owner-color-3"
        );
    }

    #[test]
    fn minesweeper_cell_aria_describes_common_states() {
        assert_eq!(
            minesweeper_cell_aria(
                1,
                2,
                MinesweeperCellDisplay {
                    flagged: true,
                    ..MinesweeperCellDisplay::default()
                }
            ),
            "Row 2, column 3, flagged"
        );
    }
}
