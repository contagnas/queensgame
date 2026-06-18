pub use queensgame_shared_minesweeper::*;
pub use queensgame_shared_nonogram::*;
pub use queensgame_shared_queens::*;
pub use queensgame_shared_room::*;
pub use queensgame_shared_room_minesweeper::*;

pub const DISPLAY_NAME_MAX_CHARS: usize = 32;

#[must_use]
pub fn normalize_display_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(DISPLAY_NAME_MAX_CHARS).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_are_trimmed_and_required() {
        assert_eq!(normalize_display_name("  Ada  "), Some("Ada".to_string()));
        assert_eq!(normalize_display_name("   "), None);

        let long_name = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJK";
        let normalized = normalize_display_name(long_name).expect("name is not empty");
        assert_eq!(normalized.chars().count(), DISPLAY_NAME_MAX_CHARS);
    }
}
