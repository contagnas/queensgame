use queensgame_shared_room::{
    ROOM_MOUSE_EVENT_PRIMARY_DOWN, ROOM_MOUSE_EVENT_PRIMARY_UP, ROOM_MOUSE_EVENT_SECONDARY_DOWN,
    ROOM_MOUSE_EVENT_SECONDARY_UP, RoomLivePointer, RoomMouseRecording,
};

pub const ROOM_BOARD_ID: &str = "room-board";

pub struct ReplayMousePointer {
    pub x_percent: String,
    pub y_percent: String,
    pub active_click: bool,
}

pub fn replay_mouse_pointer(
    recording: &RoomMouseRecording,
    replay_time_ms: u64,
) -> Option<ReplayMousePointer> {
    let replay_time_ms = u32::try_from(replay_time_ms).unwrap_or(u32::MAX);
    let (x, y) = interpolated_mouse_position(recording, replay_time_ms)?;
    let active_click = recording.events.iter().rev().any(|event| {
        event.0 <= replay_time_ms
            && replay_time_ms.saturating_sub(event.0) <= 180
            && matches!(
                event.1,
                ROOM_MOUSE_EVENT_PRIMARY_DOWN
                    | ROOM_MOUSE_EVENT_PRIMARY_UP
                    | ROOM_MOUSE_EVENT_SECONDARY_DOWN
                    | ROOM_MOUSE_EVENT_SECONDARY_UP
            )
    });

    Some(ReplayMousePointer {
        x_percent: format!("{:.3}%", x / f64::from(u16::MAX) * 100.0),
        y_percent: format!("{:.3}%", y / f64::from(u16::MAX) * 100.0),
        active_click,
    })
}

pub fn replay_mouse_class(active_click: bool, is_playing: bool) -> &'static str {
    match (active_click, is_playing) {
        (true, true) => "replay-mouse active playing",
        (true, false) => "replay-mouse active",
        (false, true) => "replay-mouse playing",
        (false, false) => "replay-mouse",
    }
}

pub fn normalized_board_pointer(client_x: f64, client_y: f64) -> Option<(u16, u16)> {
    let document = web_sys::window()?.document()?;
    let board = document.get_element_by_id(ROOM_BOARD_ID)?;
    let rect = board.get_bounding_client_rect();
    let width = rect.width();
    let height = rect.height();
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let x = ((client_x - rect.left()) / width).clamp(0.0, 1.0);
    let y = ((client_y - rect.top()) / height).clamp(0.0, 1.0);
    Some((normalized_pointer_axis(x), normalized_pointer_axis(y)))
}

pub fn normalized_pointer_axis(value: f64) -> u16 {
    (value * u16::MAX as f64)
        .round()
        .clamp(0.0, u16::MAX as f64) as u16
}

pub fn room_live_pointer_is_fresh(pointer: RoomLivePointer) -> bool {
    (now_ms() as u64).saturating_sub(pointer.updated_at_ms) <= 5_000
}

pub fn room_live_pointer_class(active_click: bool, color_index: Option<u8>) -> String {
    let mut class_name = String::from("replay-mouse room-live-pointer");
    if active_click {
        class_name.push_str(" active");
    }
    class_name.push_str(" playing");
    if let Some(color_index) = color_index {
        class_name.push_str(&format!(" player-color-{color_index}"));
    }
    class_name
}

pub fn room_live_pointer_style(pointer: RoomLivePointer) -> String {
    format!(
        "--mouse-x: {:.3}%; --mouse-y: {:.3}%",
        f64::from(pointer.x) / f64::from(u16::MAX) * 100.0,
        f64::from(pointer.y) / f64::from(u16::MAX) * 100.0
    )
}

fn interpolated_mouse_position(
    recording: &RoomMouseRecording,
    replay_time_ms: u32,
) -> Option<(f64, f64)> {
    let mut previous = None;
    let mut next = None;
    for sample in &recording.samples {
        if sample.0 <= replay_time_ms {
            previous = Some(*sample);
        } else {
            next = Some(*sample);
            break;
        }
    }

    match (previous, next) {
        (Some(previous), Some(next)) if next.0 > previous.0 => {
            let progress = f64::from(replay_time_ms.saturating_sub(previous.0))
                / f64::from(next.0 - previous.0);
            Some((
                lerp_u16(previous.1, next.1, progress),
                lerp_u16(previous.2, next.2, progress),
            ))
        }
        (Some(sample), _) | (None, Some(sample)) => {
            Some((f64::from(sample.1), f64::from(sample.2)))
        }
        (None, None) => recording
            .events
            .iter()
            .take_while(|event| event.0 <= replay_time_ms)
            .last()
            .map(|event| (f64::from(event.2), f64::from(event.3))),
    }
}

fn lerp_u16(start: u16, end: u16, progress: f64) -> f64 {
    f64::from(start) + (f64::from(end) - f64::from(start)) * progress.clamp(0.0, 1.0)
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}
