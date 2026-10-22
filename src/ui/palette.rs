use egui::Color32;

use crate::domain::{DictionaryTag, ThemeMode};

pub const WARM_PANEL: Color32 = Color32::from_rgb(244, 235, 220);
pub const WARM_WINDOW: Color32 = Color32::from_rgb(251, 245, 234);
pub const WARM_TEXT_EDIT: Color32 = Color32::from_rgb(255, 251, 243);
pub const WARM_FAINT: Color32 = Color32::from_rgb(249, 242, 230);
pub const WARM_CODE: Color32 = Color32::from_rgb(238, 227, 210);
pub const WARM_BORDER: Color32 = Color32::from_rgb(215, 198, 176);
pub const WARM_WIDGET: Color32 = Color32::from_rgb(234, 223, 206);
pub const WARM_TEXT: Color32 = Color32::from_rgb(62, 52, 40);

pub fn tag_color(tag: DictionaryTag, theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => match tag {
            DictionaryTag::NumbersAndDates => Color32::from_rgb(90, 175, 250),
            DictionaryTag::YoFication => Color32::from_rgb(100, 220, 160),
            DictionaryTag::HomographsCap => Color32::from_rgb(195, 135, 255),
            DictionaryTag::CapToPlusConverter => Color32::from_rgb(255, 180, 70),
            DictionaryTag::PlusStressWords => Color32::from_rgb(255, 125, 110),
            DictionaryTag::PlusStressOrthoepy => Color32::from_rgb(255, 95, 95),
            DictionaryTag::CleanupAndTranslit => Color32::from_rgb(150, 200, 210),
        },
        ThemeMode::Light => match tag {
            DictionaryTag::NumbersAndDates => Color32::from_rgb(30, 111, 184),
            DictionaryTag::YoFication => Color32::from_rgb(31, 122, 77),
            DictionaryTag::HomographsCap => Color32::from_rgb(122, 62, 175),
            DictionaryTag::CapToPlusConverter => Color32::from_rgb(166, 90, 0),
            DictionaryTag::PlusStressWords => Color32::from_rgb(176, 58, 46),
            DictionaryTag::PlusStressOrthoepy => Color32::from_rgb(176, 42, 42),
            DictionaryTag::CleanupAndTranslit => Color32::from_rgb(42, 107, 120),
        },
        ThemeMode::Warm => match tag {
            DictionaryTag::NumbersAndDates => Color32::from_rgb(25, 82, 140),
            DictionaryTag::YoFication => Color32::from_rgb(30, 95, 60),
            DictionaryTag::HomographsCap => Color32::from_rgb(96, 48, 135),
            DictionaryTag::CapToPlusConverter => Color32::from_rgb(128, 68, 0),
            DictionaryTag::PlusStressWords => Color32::from_rgb(140, 44, 34),
            DictionaryTag::PlusStressOrthoepy => Color32::from_rgb(140, 34, 34),
            DictionaryTag::CleanupAndTranslit => Color32::from_rgb(32, 82, 92),
        },
    }
}

pub fn weak_text(theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => Color32::from_gray(160),
        ThemeMode::Light => Color32::from_gray(105),
        ThemeMode::Warm => Color32::from_rgb(107, 91, 72),
    }
}

pub fn success_text(theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => Color32::from_rgb(100, 225, 135),
        ThemeMode::Light => Color32::from_rgb(31, 122, 77),
        ThemeMode::Warm => Color32::from_rgb(47, 120, 80),
    }
}

pub fn warning_text(theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => Color32::from_rgb(255, 180, 70),
        ThemeMode::Light => Color32::from_rgb(166, 90, 0),
        ThemeMode::Warm => Color32::from_rgb(140, 75, 0),
    }
}

pub fn diff_before(theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => Color32::from_rgb(240, 120, 120),
        ThemeMode::Light => Color32::from_rgb(176, 42, 42),
        ThemeMode::Warm => Color32::from_rgb(150, 48, 38),
    }
}

pub fn diff_after(theme: ThemeMode) -> Color32 {
    match theme {
        ThemeMode::Dark => Color32::from_rgb(110, 225, 140),
        ThemeMode::Light => Color32::from_rgb(31, 122, 77),
        ThemeMode::Warm => Color32::from_rgb(47, 120, 80),
    }
}

pub fn primary_button(theme: ThemeMode) -> (Color32, Color32) {
    match theme {
        ThemeMode::Dark | ThemeMode::Light => {
            (Color32::from_rgb(90, 215, 120), Color32::from_rgb(15, 25, 15))
        }
        ThemeMode::Warm => (
            Color32::from_rgb(156, 82, 52),
            Color32::from_rgb(255, 249, 240),
        ),
    }
}

pub fn find_highlight() -> Color32 {
    Color32::from_rgb(255, 221, 87)
}

pub fn find_current_highlight() -> Color32 {
    Color32::from_rgb(255, 183, 77)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK_PANEL: Color32 = Color32::from_gray(27);
    const LIGHT_PANEL: Color32 = Color32::from_gray(248);
    const WCAG_AA_NORMAL_TEXT: f32 = 4.5;
    const THEMES: [ThemeMode; 3] = [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Warm];

    fn panel_color(theme: ThemeMode) -> Color32 {
        match theme {
            ThemeMode::Dark => DARK_PANEL,
            ThemeMode::Light => LIGHT_PANEL,
            ThemeMode::Warm => WARM_PANEL,
        }
    }

    fn linear(channel: u8) -> f32 {
        let channel = channel as f32 / 255.0;
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: Color32) -> f32 {
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (lighter, darker) = if luminance(a) > luminance(b) {
            (luminance(a), luminance(b))
        } else {
            (luminance(b), luminance(a))
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn tag_colors_meet_wcag_aa_in_all_themes() {
        for tag in DictionaryTag::ALL {
            for theme in THEMES {
                let ratio = contrast(tag_color(tag, theme), panel_color(theme));
                assert!(
                    ratio >= WCAG_AA_NORMAL_TEXT,
                    "{tag:?} in {theme:?} has contrast {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn secondary_and_semantic_text_meets_wcag_aa_in_all_themes() {
        for theme in THEMES {
            let background = panel_color(theme);
            for (name, color) in [
                ("weak_text", weak_text(theme)),
                ("success_text", success_text(theme)),
                ("warning_text", warning_text(theme)),
                ("diff_before", diff_before(theme)),
                ("diff_after", diff_after(theme)),
            ] {
                let ratio = contrast(color, background);
                assert!(
                    ratio >= WCAG_AA_NORMAL_TEXT,
                    "{name} in {theme:?} has contrast {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn primary_button_text_meets_wcag_aa_in_all_themes() {
        for theme in THEMES {
            let (background, foreground) = primary_button(theme);
            let ratio = contrast(background, foreground);
            assert!(
                ratio >= WCAG_AA_NORMAL_TEXT,
                "primary button in {theme:?} has contrast {ratio:.2}:1"
            );
        }
    }

    #[test]
    fn warm_surface_keeps_body_text_readable() {
        let ratio = contrast(WARM_TEXT, WARM_TEXT_EDIT);
        assert!(ratio >= WCAG_AA_NORMAL_TEXT, "warm body text has {ratio:.2}:1");
    }

    #[test]
    fn contrast_helper_reports_black_and_white_as_21_to_1() {
        let ratio = contrast(Color32::WHITE, Color32::BLACK);
        assert!((ratio - 21.0).abs() < 0.01, "ratio was {ratio:.2}");
    }

    #[test]
    fn find_current_highlight_keeps_black_text_readable() {
        let ratio = contrast(Color32::BLACK, find_current_highlight());
        assert!(
            ratio >= WCAG_AA_NORMAL_TEXT,
            "current find highlight has contrast {ratio:.2}:1 with black text"
        );
    }

    #[test]
    fn find_highlight_keeps_black_text_readable() {
        let ratio = contrast(Color32::BLACK, find_highlight());
        assert!(
            ratio >= WCAG_AA_NORMAL_TEXT,
            "find highlight has contrast {ratio:.2}:1 with black text"
        );
    }
}
