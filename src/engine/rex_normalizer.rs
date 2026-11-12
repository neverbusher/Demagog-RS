/// Sanitizes Demagog / SkRegExp / PCRE patterns for Rust's linear-time `regex` engine,
/// and normalizes replacement strings so `$1` followed by Cyrillic letters is never
/// misinterpreted as a named capture group.
pub fn normalize_rex_pattern(raw: &str) -> (String, bool) {
    let mut s = raw.trim_start_matches('\u{feff}');
    let mut case_sensitive = false;

    if let Some(rest) = s.strip_prefix('@') {
        case_sensitive = true;
        s = rest;
    } else if let Some(rest) = s.strip_prefix(r"\b@") {
        return (sanitize_regex_escapes(&format!(r"\b{rest}")), true);
    }

    (sanitize_regex_escapes(s), case_sensitive)
}

fn sanitize_regex_escapes(input: &str) -> String {
    let replaced = input.replace(r"\x88", "€");
    let mut out = String::with_capacity(replaced.len() + 8);
    let mut chars = replaced.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    // Valid regex escapes and metacharacters
                    '\\' | 'b' | 'B' | 'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'n' | 'r' | 't'
                    | 'a' | 'f' | 'v' | 'x' | 'u' | 'p' | 'P' | '.' | '+' | '*' | '?' | '('
                    | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '-' => {
                        out.push('\\');
                        out.push(chars.next().unwrap());
                    }
                    // Redundant escape on literal char (e.g. \:, \,, \х, \°, \€) -> keep literal char
                    _ => {
                        out.push(chars.next().unwrap());
                    }
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Converts `$1`..`$99` into `${1}`..`${99}` so Rust's `regex` crate does not treat
/// `$1гОловы` or `$1X$2` as a named capture group `$1гОловы`.
pub fn normalize_rex_replacement(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '$' {
                out.push_str("$$");
                i += 2;
                continue;
            }
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('$');
                i += 1;
                continue;
            }
            if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end].is_ascii_digit() && (end - start) < 2 {
                    end += 1;
                }
                let num: String = chars[start..end].iter().collect();
                out.push_str("${");
                out.push_str(&num);
                out.push('}');
                i = end;
                continue;
            }
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'r' => out.push('\r'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                // A literal dollar must survive the regex crate's `$` expansion.
                '$' => out.push_str("$$"),
                other => out.push(other),
            }
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_numeric_groups_before_cyrillic() {
        assert_eq!(normalize_rex_replacement("$1гОловы"), "${1}гОловы");
        assert_eq!(normalize_rex_replacement("$2X$1"), "${2}X${1}");
        assert_eq!(normalize_rex_replacement("$$1"), "$$1");
    }

    #[test]
    fn replacement_backslash_escapes_are_unescaped() {
        assert_eq!(normalize_rex_replacement("\\$1"), "$$1");
        assert_eq!(normalize_rex_replacement("\\\\"), "\\");
        assert_eq!(normalize_rex_replacement("\\?"), "?");
        assert_eq!(normalize_rex_replacement("\\:"), ":");
    }

    #[test]
    fn keeps_at_prefix_as_case_sensitive_marker() {
        let (pattern, case_sensitive) = normalize_rex_pattern("@Слово");
        assert_eq!(pattern, "Слово");
        assert!(case_sensitive);
    }
}
