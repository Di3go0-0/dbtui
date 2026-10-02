//! Fitting a value into a fixed number of terminal cells, for single-line
//! inputs that hold more text than they can show (long hosts, long secrets).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: char = '…';

/// The end of `value`, cut to `max` cells with a leading `…` when it does not
/// fit. A focused input shows this, so the caret — which sits at the end —
/// stays on screen while typing or pasting past the edge.
pub fn fit_tail(value: &str, max: usize) -> String {
    if value.width() <= max {
        return value.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut used = 0;
    let mut tail: Vec<char> = Vec::new();
    for ch in value.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        tail.push(ch);
    }
    std::iter::once(ELLIPSIS)
        .chain(tail.into_iter().rev())
        .collect()
}

/// The start of `value`, cut to `max` cells with a trailing `…` when it does
/// not fit. An unfocused input shows this.
pub fn fit_head(value: &str, max: usize) -> String {
    if value.width() <= max {
        return value.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut used = 0;
    let mut head = String::new();
    for ch in value.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        head.push(ch);
    }
    head.push(ELLIPSIS);
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_values_are_untouched() {
        assert_eq!(fit_tail("abc", 5), "abc");
        assert_eq!(fit_head("abc", 3), "abc");
    }

    #[test]
    fn long_values_keep_the_requested_end() {
        assert_eq!(fit_tail("abcdefgh", 5), "…efgh");
        assert_eq!(fit_head("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn width_is_measured_in_cells() {
        // Each CJK character is two cells wide.
        assert_eq!(fit_tail("日本語テキスト", 7), "…キスト");
        assert_eq!(fit_head("ñandú árbol", 6), "ñandú…");
    }
}
