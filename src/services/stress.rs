use crate::domain::StressFormat;
use crate::ports::StressFormatter;

pub struct RussianStressFormatter;

impl StressFormatter for RussianStressFormatter {
    fn format_stress(&self, text: &str, format: StressFormat) -> String {
        match format {
            StressFormat::PlusBeforeVowel => text.to_string(),
            StressFormat::AcuteAccent => convert_plus_to_acute(text),
            StressFormat::CleanYoOnly => strip_stress_marks(text),
        }
    }
}

fn convert_plus_to_acute(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 128);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '+' && i + 1 < chars.len() && is_cyrillic_vowel(chars[i + 1]) {
            out.push(chars[i + 1]);
            out.push('\u{0301}');
            i += 2;
        } else if i + 1 < chars.len()
            && is_lower_cyrillic(chars[i])
            && is_upper_cyrillic_vowel(chars[i + 1])
        {
            out.push(chars[i]);
            if let Some(low) = chars[i + 1].to_lowercase().next() {
                out.push(low);
            }
            out.push('\u{0301}');
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn strip_stress_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '+' && i + 1 < chars.len() && is_cyrillic_vowel(chars[i + 1]) {
            out.push(chars[i + 1]);
            i += 2;
        } else if chars[i] == '\u{0301}' {
            i += 1;
        } else if i + 1 < chars.len()
            && is_lower_cyrillic(chars[i])
            && is_upper_cyrillic_vowel(chars[i + 1])
        {
            out.push(chars[i]);
            if let Some(low) = chars[i + 1].to_lowercase().next() {
                out.push(low);
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[inline]
fn is_cyrillic_vowel(c: char) -> bool {
    matches!(
        c,
        'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'
        | 'А' | 'Е' | 'Ё' | 'И' | 'О' | 'У' | 'Ы' | 'Э' | 'Ю' | 'Я'
    )
}

#[inline]
fn is_lower_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ё')
}

#[inline]
fn is_upper_cyrillic_vowel(c: char) -> bool {
    matches!(c, 'А' | 'Е' | 'Ё' | 'И' | 'О' | 'У' | 'Ы' | 'Э' | 'Ю' | 'Я')
}
