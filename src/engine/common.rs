//! Small helpers shared by the dictionary compilers and text processors.

/// Compile warnings are diagnostic summaries; a broken dictionary can produce
/// thousands, so keep only the first few.
pub(crate) const MAX_COMPILE_WARNINGS: usize = 50;

pub(crate) fn push_warning(warnings: &mut Vec<String>, message: impl Into<String>) {
    if warnings.len() < MAX_COMPILE_WARNINGS {
        warnings.push(message.into());
    }
}

/// Removes a UTF-8 BOM from the start of a dictionary line.
pub(crate) fn strip_bom(line: &str) -> &str {
    line.trim_start_matches('\u{feff}')
}

/// Removes a leading BOM and surrounding whitespace from a dictionary line.
pub(crate) fn trim_rule_line(line: &str) -> &str {
    strip_bom(line).trim()
}

#[inline]
pub(crate) fn is_cyrillic_letter(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ё' | 'А'..='Я' | 'Ё')
}

#[inline]
pub(crate) fn is_lower_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ё')
}

#[inline]
pub(crate) fn is_upper_cyrillic_vowel(c: char) -> bool {
    matches!(c, 'А' | 'Е' | 'Ё' | 'И' | 'О' | 'У' | 'Ы' | 'Э' | 'Ю' | 'Я')
}

#[inline]
pub(crate) fn is_cyrillic_vowel(c: char) -> bool {
    matches!(
        c,
        'а' | 'е'
            | 'ё'
            | 'и'
            | 'о'
            | 'у'
            | 'ы'
            | 'э'
            | 'ю'
            | 'я'
            | 'А'
            | 'Е'
            | 'Ё'
            | 'И'
            | 'О'
            | 'У'
            | 'Ы'
            | 'Э'
            | 'Ю'
            | 'Я'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_lines_lose_bom_and_whitespace() {
        assert_eq!(trim_rule_line("\u{feff}  кот=к+от \r"), "кот=к+от");
        assert_eq!(strip_bom("\u{feff}кот"), "кот");
    }

    #[test]
    fn compile_warnings_are_capped() {
        let mut warnings = Vec::new();
        for i in 0..MAX_COMPILE_WARNINGS + 10 {
            push_warning(&mut warnings, i.to_string());
        }
        assert_eq!(warnings.len(), MAX_COMPILE_WARNINGS);
    }

    #[test]
    fn cyrillic_predicates_are_case_aware() {
        assert!(is_cyrillic_letter('ё'));
        assert!(is_cyrillic_letter('Я'));
        assert!(!is_cyrillic_letter('a'));

        assert!(is_lower_cyrillic('я'));
        assert!(!is_lower_cyrillic('Я'));

        assert!(is_upper_cyrillic_vowel('Э'));
        assert!(!is_upper_cyrillic_vowel('Б'));

        assert!(is_cyrillic_vowel('ю'));
        assert!(is_cyrillic_vowel('Ю'));
        assert!(!is_cyrillic_vowel('й'));
    }
}
