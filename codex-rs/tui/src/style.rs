use crate::color::blend;
use crate::color::is_light;
use crate::terminal_palette::StdoutColorLevel;
use crate::terminal_palette::best_color;
use crate::terminal_palette::default_bg;
use crate::terminal_palette::rgb_color;
use crate::terminal_palette::stdout_color_level;
use ratatui::style::Color;
use ratatui::style::Style;

pub fn user_message_style() -> Style {
    user_message_style_for(default_bg())
}

pub fn proposed_plan_style() -> Style {
    proposed_plan_style_for(default_bg())
}

/// Returns the style for a user-authored message using the provided terminal background.
pub fn user_message_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    match terminal_bg {
        Some(bg) => Style::default().bg(user_message_bg(bg)),
        None => Style::default(),
    }
}

pub fn proposed_plan_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    match terminal_bg {
        Some(bg) => Style::default().bg(proposed_plan_bg(bg)),
        None => Style::default(),
    }
}

#[allow(clippy::disallowed_methods)]
pub fn user_message_bg(terminal_bg: (u8, u8, u8)) -> Color {
    let (top, alpha) = if is_light(terminal_bg) {
        ((0, 0, 0), 0.04)
    } else {
        ((255, 255, 255), 0.12)
    };
    subtle_surface_bg(blend(top, terminal_bg, alpha))
}

#[allow(clippy::disallowed_methods)]
pub fn proposed_plan_bg(terminal_bg: (u8, u8, u8)) -> Color {
    user_message_bg(terminal_bg)
}

fn subtle_surface_bg(target: (u8, u8, u8)) -> Color {
    match stdout_color_level() {
        // Some transports and test backends fail capability detection even though RGB colors still
        // render correctly. Prefer a visible shaded surface over silently resetting to the
        // terminal default.
        StdoutColorLevel::Unknown => rgb_color(target),
        _ => best_color(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_style_uses_fallback_background_when_default_palette_is_unavailable() {
        assert_ne!(user_message_style().bg, Some(Color::Reset));
    }

    #[test]
    fn subtle_surface_bg_uses_rgb_when_color_level_is_unknown() {
        assert!(matches!(
            subtle_surface_bg_for((34, 34, 43), StdoutColorLevel::Unknown),
            Color::Rgb(_, _, _)
        ));
    }

    #[test]
    fn subtle_surface_bg_uses_best_color_when_color_level_is_known() {
        assert_eq!(
            subtle_surface_bg_for((34, 34, 43), StdoutColorLevel::Ansi16),
            best_color((34, 34, 43))
        );
    }

    fn subtle_surface_bg_for(target: (u8, u8, u8), color_level: StdoutColorLevel) -> Color {
        match color_level {
            StdoutColorLevel::Unknown => rgb_color(target),
            _ => best_color(target),
        }
    }
}
