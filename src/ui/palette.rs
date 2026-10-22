use egui::Color32;

use crate::domain::DictionaryTag;

pub fn tag_color(tag: DictionaryTag, dark: bool) -> Color32 {
    if dark {
        match tag {
            DictionaryTag::NumbersAndDates => Color32::from_rgb(90, 175, 250),
            DictionaryTag::YoFication => Color32::from_rgb(100, 220, 160),
            DictionaryTag::HomographsCap => Color32::from_rgb(195, 135, 255),
            DictionaryTag::CapToPlusConverter => Color32::from_rgb(255, 180, 70),
            DictionaryTag::PlusStressWords => Color32::from_rgb(255, 125, 110),
            DictionaryTag::PlusStressOrthoepy => Color32::from_rgb(255, 95, 95),
            DictionaryTag::CleanupAndTranslit => Color32::from_rgb(150, 200, 210),
        }
    } else {
        match tag {
            DictionaryTag::NumbersAndDates => Color32::from_rgb(30, 111, 184),
            DictionaryTag::YoFication => Color32::from_rgb(31, 122, 77),
            DictionaryTag::HomographsCap => Color32::from_rgb(122, 62, 175),
            DictionaryTag::CapToPlusConverter => Color32::from_rgb(166, 90, 0),
            DictionaryTag::PlusStressWords => Color32::from_rgb(176, 58, 46),
            DictionaryTag::PlusStressOrthoepy => Color32::from_rgb(176, 42, 42),
            DictionaryTag::CleanupAndTranslit => Color32::from_rgb(42, 107, 120),
        }
    }
}

pub fn weak_text(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(160)
    } else {
        Color32::from_gray(105)
    }
}

pub fn success_text(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(100, 225, 135)
    } else {
        Color32::from_rgb(31, 122, 77)
    }
}

pub fn warning_text(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(255, 180, 70)
    } else {
        Color32::from_rgb(166, 90, 0)
    }
}

pub fn diff_before(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(240, 120, 120)
    } else {
        Color32::from_rgb(176, 42, 42)
    }
}

pub fn diff_after(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(110, 225, 140)
    } else {
        Color32::from_rgb(31, 122, 77)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK_PANEL: Color32 = Color32::from_gray(27);
    const LIGHT_PANEL: Color32 = Color32::from_gray(248);
    const WCAG_AA_NORMAL_TEXT: f32 = 4.5;

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
    fn tag_colors_meet_wcag_aa_in_both_themes() {
        for tag in DictionaryTag::ALL {
            for (dark, background) in [(true, DARK_PANEL), (false, LIGHT_PANEL)] {
                let ratio = contrast(tag_color(tag, dark), background);
                assert!(
                    ratio >= WCAG_AA_NORMAL_TEXT,
                    "{tag:?} in dark={dark} has contrast {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn secondary_and_semantic_text_meets_wcag_aa_in_both_themes() {
        for dark in [true, false] {
            let background = if dark { DARK_PANEL } else { LIGHT_PANEL };
            for (name, color) in [
                ("weak_text", weak_text(dark)),
                ("success_text", success_text(dark)),
                ("warning_text", warning_text(dark)),
                ("diff_before", diff_before(dark)),
                ("diff_after", diff_after(dark)),
            ] {
                let ratio = contrast(color, background);
                assert!(
                    ratio >= WCAG_AA_NORMAL_TEXT,
                    "{name} in dark={dark} has contrast {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn contrast_helper_reports_black_and_white_as_21_to_1() {
        let ratio = contrast(Color32::WHITE, Color32::BLACK);
        assert!((ratio - 21.0).abs() < 0.01, "ratio was {ratio:.2}");
    }
}
