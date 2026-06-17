use queensgame_server_rooms_model::Room;
use queensgame_shared_queens::{CellState, Puzzle};
use queensgame_shared_room::{
    ROOM_MOUSE_EVENT_ENTER, ROOM_MOUSE_EVENT_LEAVE, ROOM_MOUSE_EVENT_PRIMARY_DOWN,
    ROOM_MOUSE_EVENT_PRIMARY_UP, ROOM_MOUSE_EVENT_SECONDARY_DOWN, ROOM_MOUSE_EVENT_SECONDARY_UP,
    RoomMouseRecording, RoomRecording, mouse_recording_times_are_sorted, recording_frame_is_valid,
};

pub const MAX_RECORDING_FRAMES: usize = 10_000;
pub const MAX_MOUSE_SAMPLES: usize = 100_000;
pub const MAX_MOUSE_EVENTS: usize = 100_000;

#[must_use]
pub fn recording_matches_solution(
    puzzle: &Puzzle,
    queens: &[[usize; 2]],
    recording: &RoomRecording,
) -> bool {
    let Some(last_frame) = recording.frames.last() else {
        return false;
    };
    let cell_count = puzzle.size * puzzle.size;
    if recording
        .frames
        .iter()
        .any(|frame| !recording_frame_is_valid(frame, cell_count))
    {
        return false;
    }
    if recording
        .frames
        .windows(2)
        .any(|frames| frames[0].elapsed_ms > frames[1].elapsed_ms)
    {
        return false;
    }

    let recorded_queens: Vec<[usize; 2]> = last_frame
        .states
        .iter()
        .enumerate()
        .filter_map(|(index, state)| {
            if CellState::from_storage_code(*state) == CellState::Queen {
                Some([index / puzzle.size, index % puzzle.size])
            } else {
                None
            }
        })
        .collect();
    let mut recorded_queens = recorded_queens;
    let mut submitted_queens = queens.to_vec();
    recorded_queens.sort_unstable();
    submitted_queens.sort_unstable();
    recorded_queens == submitted_queens
}

#[must_use]
pub fn mouse_recording_is_valid(puzzle: &Puzzle, recording: &RoomMouseRecording) -> bool {
    if recording.samples.len() > MAX_MOUSE_SAMPLES || recording.events.len() > MAX_MOUSE_EVENTS {
        return false;
    }
    if !mouse_recording_times_are_sorted(recording) {
        return false;
    }

    let cell_count = puzzle.size.saturating_mul(puzzle.size);
    recording.events.iter().all(|event| {
        matches!(
            event.1,
            ROOM_MOUSE_EVENT_ENTER
                | ROOM_MOUSE_EVENT_LEAVE
                | ROOM_MOUSE_EVENT_PRIMARY_DOWN
                | ROOM_MOUSE_EVENT_PRIMARY_UP
                | ROOM_MOUSE_EVENT_SECONDARY_DOWN
                | ROOM_MOUSE_EVENT_SECONDARY_UP
        ) && event
            .4
            .is_none_or(|cell_index| usize::from(cell_index) < cell_count)
    })
}

pub fn award_room_queens_medals(room: &mut Room) {
    let mut placements = room
        .race_player_ids
        .iter()
        .filter_map(|player_id| {
            let player = room.players.get(player_id)?;
            player
                .finish_ms
                .map(|finish_ms| (player.id.clone(), finish_ms, player.joined_order))
        })
        .collect::<Vec<_>>();
    placements.sort_by_key(|(_, finish_ms, joined_order)| (*finish_ms, *joined_order));

    for (place, (player_id, _, _)) in placements.into_iter().take(3).enumerate() {
        let Some(player) = room.players.get_mut(&player_id) else {
            continue;
        };
        match place {
            0 => player.medals.gold += 1,
            1 => player.medals.silver += 1,
            2 => player.medals.bronze += 1,
            _ => {}
        }
    }
}
