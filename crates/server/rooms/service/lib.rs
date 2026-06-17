#![allow(clippy::missing_errors_doc)]
#![allow(clippy::significant_drop_tightening)]

use nanoid::nanoid;
use queensgame_server_rooms_error::RoomError;
use queensgame_server_rooms_minesweeper::{
    active_minesweeper_elapsed_ms, apply_minesweeper_player_result, award_room_minesweeper_medals,
    claim_minesweeper_revealed_cells, detonate_room_minesweeper_mine,
    prepare_room_minesweeper_game, reveal_room_minesweeper_starts, room_minesweeper_chord_flags,
    room_minesweeper_player_can_act, room_minesweeper_player_can_point,
    room_minesweeper_should_complete,
};
use queensgame_server_rooms_model::{
    AppState, Room, RoomPlayer, ServerRoomPhase, begin_room_race_for_connected_players,
    clear_room_race_results, elapsed_millis_u64, now_ms, require, reset_room_ready_flags,
    reset_room_setup_for_selection, room_accepts_next_race_setup, room_all_connected_players_ready,
    room_all_racers_done, with_room, with_room_mut,
};
use queensgame_server_rooms_queens::{
    MAX_MOUSE_EVENTS, MAX_MOUSE_SAMPLES, MAX_RECORDING_FRAMES, award_room_queens_medals,
    mouse_recording_is_valid, recording_matches_solution,
};
use queensgame_server_rooms_snapshot::{
    pending_room_server_message, pending_room_snapshot, room_snapshot_message,
    send_pending_room_message, send_room_error_locked, snapshot_room,
};
use queensgame_shared_minesweeper::{
    MinesweeperCellState, clamp_room_minesweeper_tile_axis,
    clamp_room_minesweeper_time_limit_seconds,
};
use queensgame_shared_queens::{Puzzle, find_puzzle_by_id, next_puzzle_id, validate_solution};
use queensgame_shared_room::{
    RoomBootstrap, RoomClientMessage, RoomGameKind, RoomLivePointer, RoomMouseRecording,
    RoomPuzzleChoice, RoomRecording, RoomRecordingFrame, RoomServerMessage, append_mouse_recording,
    append_recording_frame, recording_frame_is_valid,
};
use rand::Rng;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub async fn create_room(state: &AppState) -> String {
    {
        let mut rooms = state.rooms.lock().await;
        let slug = loop {
            let candidate = nanoid!(8, &nanoid::alphabet::SAFE);
            if !rooms.contains_key(&candidate) {
                break candidate;
            }
        };
        let (tx, _) = broadcast::channel(64);
        rooms.insert(slug.clone(), Room::new(slug.clone(), tx));
        slug
    }
}

pub async fn join_room(
    state: &AppState,
    slug: &str,
    player_id: &str,
    player_name: String,
) -> Option<(String, broadcast::Receiver<String>)> {
    let (initial_snapshot, rx, broadcast) = {
        let mut rooms = state.rooms.lock().await;
        let room = rooms.get_mut(slug)?;
        let joined_order = room.players.len() as u64 + 1;
        let reset_ready = matches!(room.phase, ServerRoomPhase::Lobby);
        room.players
            .entry(player_id.to_string())
            .and_modify(|player| {
                player.name.clone_from(&player_name);
                player.connected = true;
                if reset_ready {
                    player.ready = false;
                }
            })
            .or_insert_with(|| RoomPlayer::new(player_id.to_string(), player_name, joined_order));
        let initial_snapshot = room_snapshot_message(room, &state.puzzles);
        let broadcast = pending_room_server_message(
            room,
            &RoomServerMessage::Snapshot {
                snapshot: snapshot_room(room, &state.puzzles),
            },
        );
        let rx = room.tx.subscribe();
        (initial_snapshot, rx, broadcast)
    };
    broadcast.send();

    Some((initial_snapshot, rx))
}

pub async fn disconnect_player(state: &AppState, slug: &str, player_id: &str) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            if let Some(player) = room.players.get_mut(player_id) {
                player.connected = false;
                player.pointer = None;
                if matches!(room.phase, ServerRoomPhase::Lobby) {
                    player.ready = false;
                }
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

pub async fn handle_room_message(
    state: &AppState,
    slug: &str,
    player_id: &str,
    message: RoomClientMessage,
) {
    match message {
        RoomClientMessage::SelectGame { game_kind } => {
            select_room_game(state, slug, game_kind).await;
        }
        RoomClientMessage::SelectPuzzle { puzzle_id } => {
            select_room_puzzle(state, slug, puzzle_id).await;
        }
        RoomClientMessage::SelectRandom => {
            select_random_puzzle(state, slug).await;
        }
        RoomClientMessage::SetMinesweeperTimeLimit { seconds } => {
            set_room_minesweeper_time_limit(state, slug, seconds).await;
        }
        RoomClientMessage::SetMinesweeperTiles { rows, cols } => {
            set_room_minesweeper_tiles(state, slug, rows, cols).await;
        }
        RoomClientMessage::SetReady { ready } => {
            set_player_ready(state, slug, player_id, ready).await;
        }
        RoomClientMessage::Finish { queens, recording } => {
            finish_player(state, slug, player_id, queens, recording).await;
        }
        RoomClientMessage::GiveUp => {
            give_up_player(state, slug, player_id).await;
        }
        RoomClientMessage::RecordingFrame { frame } => {
            store_recording_frame(state, slug, player_id, frame).await;
        }
        RoomClientMessage::MouseRecordingChunk { recording } => {
            store_mouse_recording_chunk(state, slug, player_id, recording).await;
        }
        RoomClientMessage::MouseRecording { recording } => {
            store_mouse_recording(state, slug, player_id, recording).await;
        }
        RoomClientMessage::MinesweeperReveal { index } => {
            reveal_room_minesweeper_cell(state, slug, player_id, index).await;
        }
        RoomClientMessage::MinesweeperToggleFlag { index } => {
            toggle_room_minesweeper_flag(state, slug, player_id, index).await;
        }
        RoomClientMessage::MinesweeperChord { index } => {
            chord_room_minesweeper_cell_for_player(state, slug, player_id, index).await;
        }
        RoomClientMessage::PointerUpdate { pointer } => {
            update_room_pointer(state, slug, player_id, pointer).await;
        }
    }
}

async fn select_room_game(state: &AppState, slug: &str, game_kind: RoomGameKind) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room_accepts_next_race_setup(room))?;
            if room.game_kind != game_kind {
                room.game_kind = game_kind;
                clear_room_race_results(room);
                room.phase = ServerRoomPhase::Lobby;
                reset_room_ready_flags(room);
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn select_room_puzzle(state: &AppState, slug: &str, puzzle_id: usize) {
    if find_puzzle_by_id(&state.puzzles, puzzle_id).is_none() {
        send_room_error(state, slug, format!("Puzzle {puzzle_id} does not exist.")).await;
        return;
    }

    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens && room_accepts_next_race_setup(room))?;
            room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: puzzle_id };
            reset_room_setup_for_selection(room);
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn set_room_minesweeper_time_limit(state: &AppState, slug: &str, seconds: u32) {
    update_room_minesweeper_setup(state, slug, |room| {
        room.minesweeper_time_limit_seconds = clamp_room_minesweeper_time_limit_seconds(seconds);
    })
    .await;
}

async fn set_room_minesweeper_tiles(state: &AppState, slug: &str, rows: usize, cols: usize) {
    update_room_minesweeper_setup(state, slug, |room| {
        room.minesweeper_tile_rows = clamp_room_minesweeper_tile_axis(rows);
        room.minesweeper_tile_cols = clamp_room_minesweeper_tile_axis(cols);
    })
    .await;
}

async fn update_room_minesweeper_setup(
    state: &AppState,
    slug: &str,
    update: impl FnOnce(&mut Room),
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(
                room.game_kind == RoomGameKind::Minesweeper && room_accepts_next_race_setup(room),
            )?;
            update(room);
            reset_room_ready_flags(room);
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn select_random_puzzle(state: &AppState, slug: &str) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens && room_accepts_next_race_setup(room))?;
            room.puzzle_choice = RoomPuzzleChoice::Random;
            reset_room_setup_for_selection(room);
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn set_player_ready(state: &AppState, slug: &str, player_id: &str, ready: bool) {
    let result = {
        let mut starts_at_ms = None;
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room_accepts_next_race_setup(room))?;
            if let Some(player) = room.players.get_mut(player_id) {
                player.ready = ready;
            }

            if room_all_connected_players_ready(room) {
                clear_room_race_results(room);
                if room.game_kind == RoomGameKind::Minesweeper {
                    prepare_room_minesweeper_game(room);
                }
                let countdown_ms = if room.game_kind == RoomGameKind::Minesweeper {
                    5_000
                } else {
                    3_000
                };
                let start = now_ms() + countdown_ms;
                room.phase = ServerRoomPhase::Countdown {
                    starts_at_ms: start,
                };
                starts_at_ms = Some(start);
            }
            Some((starts_at_ms, pending_room_snapshot(room, &state.puzzles)))
        })
    };
    let Some((starts_at_ms, broadcast)) = result else {
        return;
    };
    broadcast.send();

    if let Some(starts_at_ms) = starts_at_ms {
        schedule_room_start(state.clone(), slug.to_string(), starts_at_ms);
    }
}

fn schedule_room_start(state: AppState, slug: String, starts_at_ms: u64) {
    tokio::spawn(async move {
        let delay_ms = starts_at_ms.saturating_sub(now_ms());
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;

        let result = {
            let mut rooms = state.rooms.lock().await;
            with_room_mut(&mut rooms, &slug, |room| {
                require(matches!(
                    room.phase,
                    ServerRoomPhase::Countdown {
                        starts_at_ms: active_start
                    } if active_start == starts_at_ms
                ))?;

                match room.game_kind {
                    RoomGameKind::Queens => {
                        let puzzle_id = match room.puzzle_choice {
                            RoomPuzzleChoice::Puzzle { id } => id,
                            RoomPuzzleChoice::Random => {
                                random_room_puzzle_id(&state.puzzles, &room.played_puzzle_ids)?
                            }
                        };
                        room.active_puzzle_id = Some(puzzle_id);
                        begin_room_race_for_connected_players(room);
                    }
                    RoomGameKind::Minesweeper => {
                        if room.minesweeper.is_none() {
                            prepare_room_minesweeper_game(room);
                        }
                        reveal_room_minesweeper_starts(room);
                        begin_room_race_for_connected_players(room);
                    }
                }

                room.phase = ServerRoomPhase::Racing {
                    started_at_ms: now_ms(),
                    started_at: Instant::now(),
                };
                let timeout = if room.game_kind == RoomGameKind::Minesweeper {
                    Some((
                        room.phase
                            .as_snapshot_phase()
                            .race_started_at_ms()
                            .unwrap_or_default(),
                        room.minesweeper_time_limit_seconds,
                    ))
                } else {
                    None
                };
                Some((timeout, pending_room_snapshot(room, &state.puzzles)))
            })
        };
        let Some((timeout, broadcast)) = result else {
            return;
        };

        if let Some((started_at_ms, seconds)) = timeout {
            schedule_room_minesweeper_timeout(state.clone(), slug.clone(), started_at_ms, seconds);
        }
        broadcast.send();
    });
}

async fn store_recording_frame(
    state: &AppState,
    slug: &str,
    player_id: &str,
    frame: RoomRecordingFrame,
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens)?;
            require(matches!(room.phase, ServerRoomPhase::Racing { .. }))?;
            require(room.race_player_ids.iter().any(|id| id == player_id))?;
            let puzzle = find_puzzle_by_id(&state.puzzles, room.active_puzzle_id?)?;
            require(recording_frame_is_valid(
                &frame,
                puzzle.size.saturating_mul(puzzle.size),
            ))?;

            let player = room.players.get_mut(player_id)?;
            require(player.finish_ms.is_none() && !player.gave_up)?;
            let recording = player
                .recording
                .get_or_insert_with(|| RoomRecording { frames: Vec::new() });
            require(recording.frames.len() < MAX_RECORDING_FRAMES)?;
            let broadcast_frame = frame.clone();
            require(append_recording_frame(recording, frame))?;

            Some(pending_room_server_message(
                room,
                &RoomServerMessage::RecordingFrame {
                    player_id: player_id.to_string(),
                    frame: broadcast_frame,
                },
            ))
        })
    };
    send_pending_room_message(broadcast);
}

async fn finish_player(
    state: &AppState,
    slug: &str,
    player_id: &str,
    queens: Vec<[usize; 2]>,
    recording: RoomRecording,
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens)?;
            let (started_at_ms, elapsed_ms) = match &room.phase {
                ServerRoomPhase::Racing {
                    started_at_ms,
                    started_at,
                } => (*started_at_ms, elapsed_millis_u64(*started_at)),
                ServerRoomPhase::Lobby
                | ServerRoomPhase::Countdown { .. }
                | ServerRoomPhase::Complete { .. } => return None,
            };
            let puzzle_id = room.active_puzzle_id?;
            let puzzle = find_puzzle_by_id(&state.puzzles, puzzle_id)?;
            if !validate_solution(puzzle, &queens).complete {
                send_room_error_locked(
                    room,
                    &format!("Submitted solution for puzzle {puzzle_id} is not complete."),
                );
                return None;
            }
            if !recording_matches_solution(puzzle, &queens, &recording) {
                send_room_error_locked(room, "Submitted replay does not match the finished board.");
                return None;
            }

            if let Some(player) = room.players.get_mut(player_id)
                && player.finish_ms.is_none()
                && !player.gave_up
            {
                player.finish_ms = Some(elapsed_ms);
                player.recording = Some(recording);
            }

            if room_all_racers_done(room) {
                complete_room_race(room, &state.puzzles, started_at_ms);
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn give_up_player(state: &AppState, slug: &str, player_id: &str) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            let started_at_ms = match &room.phase {
                ServerRoomPhase::Racing { started_at_ms, .. } => *started_at_ms,
                ServerRoomPhase::Lobby
                | ServerRoomPhase::Countdown { .. }
                | ServerRoomPhase::Complete { .. } => return None,
            };
            require(room.race_player_ids.iter().any(|id| id == player_id))?;

            match room.game_kind {
                RoomGameKind::Queens => {
                    if let Some(player) = room.players.get_mut(player_id)
                        && player.finish_ms.is_none()
                    {
                        player.gave_up = true;
                    }

                    if room_all_racers_done(room) {
                        complete_room_race(room, &state.puzzles, started_at_ms);
                    }
                }
                RoomGameKind::Minesweeper => {
                    if let Some(player) = room.players.get_mut(player_id) {
                        player.minesweeper_eliminated = true;
                        player.pointer = None;
                    }
                    if room_minesweeper_should_complete(room) {
                        complete_room_race(room, &state.puzzles, started_at_ms);
                    }
                }
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn store_mouse_recording_chunk(
    state: &AppState,
    slug: &str,
    player_id: &str,
    recording: RoomMouseRecording,
) {
    if recording.samples.is_empty() && recording.events.is_empty() {
        return;
    }

    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens)?;
            require(matches!(room.phase, ServerRoomPhase::Racing { .. }))?;
            require(room.race_player_ids.iter().any(|id| id == player_id))?;
            let puzzle = find_puzzle_by_id(&state.puzzles, room.active_puzzle_id?)?;
            require(mouse_recording_is_valid(puzzle, &recording))?;

            let player = room.players.get_mut(player_id)?;
            require(player.finish_ms.is_none() && !player.gave_up)?;
            let existing = player
                .mouse_recording
                .get_or_insert_with(|| RoomMouseRecording {
                    samples: Vec::new(),
                    events: Vec::new(),
                });
            require(
                existing
                    .samples
                    .len()
                    .saturating_add(recording.samples.len())
                    <= MAX_MOUSE_SAMPLES
                    && existing.events.len().saturating_add(recording.events.len())
                        <= MAX_MOUSE_EVENTS,
            )?;

            let broadcast_recording = recording.clone();
            require(append_mouse_recording(existing, recording))?;

            Some(pending_room_server_message(
                room,
                &RoomServerMessage::MouseRecordingChunk {
                    player_id: player_id.to_string(),
                    recording: broadcast_recording,
                },
            ))
        })
    };
    send_pending_room_message(broadcast);
}

async fn store_mouse_recording(
    state: &AppState,
    slug: &str,
    player_id: &str,
    recording: RoomMouseRecording,
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Queens)?;
            let puzzle = find_puzzle_by_id(&state.puzzles, room.active_puzzle_id?)?;
            if !mouse_recording_is_valid(puzzle, &recording) {
                send_room_error_locked(room, "Submitted mouse replay data is invalid.");
                return None;
            }
            let player = room.players.get_mut(player_id)?;
            require(player.finish_ms.is_some())?;
            player.mouse_recording = Some(recording);
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn reveal_room_minesweeper_cell(state: &AppState, slug: &str, player_id: &str, index: usize) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            let (started_at_ms, elapsed_ms) = active_minesweeper_elapsed_ms(room)?;
            require(room_minesweeper_player_can_act(room, player_id))?;
            require(
                !room
                    .players
                    .get(player_id)
                    .is_some_and(|player| player.minesweeper_flags.contains(&index)),
            )?;
            let game = room.minesweeper.as_mut()?;
            let cell = game.board.cells.get(index)?;
            require(cell.state != MinesweeperCellState::Revealed)?;

            let (score_delta, eliminated) = if cell.mine {
                detonate_room_minesweeper_mine(game, player_id, index);
                (0, true)
            } else {
                let revealed = game.board.reveal_safe_cells(index);
                (
                    claim_minesweeper_revealed_cells(game, player_id, &revealed),
                    false,
                )
            };

            apply_minesweeper_player_result(room, player_id, score_delta, elapsed_ms, eliminated);
            if room_minesweeper_should_complete(room) {
                complete_room_race(room, &state.puzzles, started_at_ms);
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn toggle_room_minesweeper_flag(state: &AppState, slug: &str, player_id: &str, index: usize) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            active_minesweeper_elapsed_ms(room)?;
            require(room_minesweeper_player_can_act(room, player_id))?;
            let game = room.minesweeper.as_ref()?;
            let cell = game.board.cells.get(index)?;
            require(cell.state != MinesweeperCellState::Revealed)?;
            let player = room.players.get_mut(player_id)?;
            if !player.minesweeper_flags.remove(&index) {
                player.minesweeper_flags.insert(index);
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn chord_room_minesweeper_cell_for_player(
    state: &AppState,
    slug: &str,
    player_id: &str,
    index: usize,
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            let (started_at_ms, elapsed_ms) = active_minesweeper_elapsed_ms(room)?;
            require(room_minesweeper_player_can_act(room, player_id))?;
            let own_flags = room
                .players
                .get(player_id)
                .map(|player| player.minesweeper_flags.clone())
                .unwrap_or_default();
            let game = room.minesweeper.as_mut()?;
            let cell = game.board.cells.get(index)?;
            require(
                cell.state == MinesweeperCellState::Revealed
                    && !cell.mine
                    && cell.adjacent_mines > 0,
            )?;
            let neighbors = game.board.neighbors(index);
            let flags = room_minesweeper_chord_flags(game, &own_flags, player_id);
            let flagged_neighbors = neighbors
                .iter()
                .filter(|neighbor| flags.contains(neighbor))
                .count();
            require(flagged_neighbors == usize::from(cell.adjacent_mines))?;

            let targets = neighbors
                .into_iter()
                .filter(|neighbor| !flags.contains(neighbor))
                .filter(|neighbor| {
                    game.board.cells[*neighbor].state != MinesweeperCellState::Revealed
                })
                .collect::<Vec<_>>();
            let mut eliminated = false;
            let mut score_delta = 0u32;
            if let Some(mine) = targets
                .iter()
                .copied()
                .find(|target| game.board.cells[*target].mine)
            {
                detonate_room_minesweeper_mine(game, player_id, mine);
                eliminated = true;
            } else {
                for target in targets {
                    let revealed = game.board.reveal_safe_cells(target);
                    score_delta = score_delta.saturating_add(claim_minesweeper_revealed_cells(
                        game, player_id, &revealed,
                    ));
                }
            }

            apply_minesweeper_player_result(room, player_id, score_delta, elapsed_ms, eliminated);
            if room_minesweeper_should_complete(room) {
                complete_room_race(room, &state.puzzles, started_at_ms);
            }
            Some(pending_room_snapshot(room, &state.puzzles))
        })
    };
    send_pending_room_message(broadcast);
}

async fn update_room_pointer(
    state: &AppState,
    slug: &str,
    player_id: &str,
    pointer: Option<RoomLivePointer>,
) {
    let broadcast = {
        let mut rooms = state.rooms.lock().await;
        with_room_mut(&mut rooms, slug, |room| {
            require(room.game_kind == RoomGameKind::Minesweeper)?;
            require(pointer.is_none() || room_minesweeper_player_can_point(room, player_id))?;
            let player = room.players.get_mut(player_id)?;
            let pointer = pointer.map(|mut pointer| {
                pointer.updated_at_ms = now_ms();
                pointer
            });
            player.pointer = pointer;
            Some(pending_room_server_message(
                room,
                &RoomServerMessage::PointerUpdate {
                    player_id: player_id.to_string(),
                    pointer,
                },
            ))
        })
    };
    send_pending_room_message(broadcast);
}

pub async fn send_room_error(state: &AppState, slug: &str, message: String) {
    let broadcast = {
        let rooms = state.rooms.lock().await;
        with_room(&rooms, slug, |room| {
            Some(pending_room_server_message(
                room,
                &RoomServerMessage::Error { message },
            ))
        })
    };
    send_pending_room_message(broadcast);
}

fn schedule_room_minesweeper_timeout(
    state: AppState,
    slug: String,
    started_at_ms: u64,
    seconds: u32,
) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
        let broadcast = {
            let mut rooms = state.rooms.lock().await;
            with_room_mut(&mut rooms, &slug, |room| {
                require(
                    matches!(
                        room.phase,
                        ServerRoomPhase::Racing {
                            started_at_ms: active_start,
                            ..
                        } if active_start == started_at_ms
                    ) && room.game_kind == RoomGameKind::Minesweeper,
                )?;
                complete_room_race(room, &state.puzzles, started_at_ms);
                Some(pending_room_snapshot(room, &state.puzzles))
            })
        };
        send_pending_room_message(broadcast);
    });
}

pub fn complete_room_race(room: &mut Room, puzzles: &[Puzzle], started_at_ms: u64) {
    match room.game_kind {
        RoomGameKind::Queens => {
            let completed_puzzle_id = room.active_puzzle_id;
            if let Some(puzzle_id) = completed_puzzle_id {
                room.played_puzzle_ids.insert(puzzle_id);
            }

            award_room_queens_medals(room);

            if let (RoomPuzzleChoice::Puzzle { .. }, Some(puzzle_id)) =
                (&room.puzzle_choice, completed_puzzle_id)
                && let Some(next_id) = next_puzzle_id(puzzles, puzzle_id)
            {
                room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: next_id };
            }
        }
        RoomGameKind::Minesweeper => {
            award_room_minesweeper_medals(room);
        }
    }

    room.phase = ServerRoomPhase::Complete { started_at_ms };
}

fn random_room_puzzle_id(
    puzzles: &[Puzzle],
    played_puzzle_ids: &std::collections::BTreeSet<usize>,
) -> Option<usize> {
    let mut candidates = puzzles
        .iter()
        .filter(|puzzle| !played_puzzle_ids.contains(&puzzle.id))
        .map(|puzzle| puzzle.id)
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        candidates = puzzles.iter().map(|puzzle| puzzle.id).collect();
    }
    if candidates.is_empty() {
        return None;
    }

    let index = rand::rng().random_range(0..candidates.len());
    candidates.get(index).copied()
}

pub async fn room_bootstrap(state: &AppState, slug: String) -> Result<RoomBootstrap, RoomError> {
    let snapshot = {
        let rooms = state.rooms.lock().await;
        rooms
            .get(&slug)
            .map(|room| snapshot_room(room, &state.puzzles))
    }
    .ok_or(RoomError::NotFound)?;

    Ok(RoomBootstrap {
        slug,
        total_puzzles: state.puzzles.len(),
        snapshot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use queensgame_server_rooms_model::test_support::{add_test_player, test_puzzle, test_room};

    #[test]
    fn completing_room_race_records_puzzle_and_awards_medals() {
        let puzzles = vec![
            test_puzzle(1),
            test_puzzle(2),
            test_puzzle(3),
            test_puzzle(4),
        ];
        let mut room = test_room(RoomPuzzleChoice::Puzzle { id: 2 });
        room.active_puzzle_id = Some(2);
        add_test_player(&mut room, "ada", Some(1_200), false, 1);
        add_test_player(&mut room, "bea", Some(900), false, 2);
        add_test_player(&mut room, "cam", None, true, 3);
        add_test_player(&mut room, "dee", Some(1_500), false, 4);

        assert!(room_all_racers_done(&room));
        complete_room_race(&mut room, &puzzles, 42);

        assert!(room.played_puzzle_ids.contains(&2));
        assert!(matches!(
            room.puzzle_choice,
            RoomPuzzleChoice::Puzzle { id: 3 }
        ));
        assert!(matches!(
            room.phase,
            ServerRoomPhase::Complete { started_at_ms: 42 }
        ));
        assert_eq!(room.players["bea"].medals.gold, 1);
        assert_eq!(room.players["ada"].medals.silver, 1);
        assert_eq!(room.players["dee"].medals.bronze, 1);
        assert_eq!(room.players["cam"].medals.total(), 0);
    }

    #[test]
    fn completing_minesweeper_race_awards_medals_by_score() {
        let puzzles = vec![test_puzzle(1)];
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        add_test_player(&mut room, "bea", None, false, 2);
        add_test_player(&mut room, "cam", None, false, 3);

        room.players.get_mut("ada").unwrap().minesweeper_score = 12;
        room.players
            .get_mut("ada")
            .unwrap()
            .minesweeper_last_score_ms = Some(800);
        room.players.get_mut("bea").unwrap().minesweeper_score = 14;
        room.players
            .get_mut("bea")
            .unwrap()
            .minesweeper_last_score_ms = Some(900);
        room.players.get_mut("cam").unwrap().minesweeper_score = 12;
        room.players
            .get_mut("cam")
            .unwrap()
            .minesweeper_last_score_ms = Some(700);

        complete_room_race(&mut room, &puzzles, 99);

        assert_eq!(room.players["bea"].medals.gold, 1);
        assert_eq!(room.players["cam"].medals.silver, 1);
        assert_eq!(room.players["ada"].medals.bronze, 1);
        assert!(matches!(
            room.phase,
            ServerRoomPhase::Complete { started_at_ms: 99 }
        ));
    }

    #[test]
    fn random_room_puzzle_id_uses_unplayed_puzzles_first() {
        let puzzles = vec![test_puzzle(1), test_puzzle(2), test_puzzle(3)];
        let played = std::collections::BTreeSet::from([1, 2]);

        assert_eq!(random_room_puzzle_id(&puzzles, &played), Some(3));

        let played = std::collections::BTreeSet::from([1, 2, 3]);
        let next = random_room_puzzle_id(&puzzles, &played);
        assert!(matches!(next, Some(1..=3)));
    }
}
