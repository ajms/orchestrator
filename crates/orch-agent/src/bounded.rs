//! Bounds text a Subagent reports from inside the Sandbox before it reaches
//! the Daemon: it is untrusted, so it is trimmed, stripped of control
//! characters and cut to a size the TUI can show.

pub(crate) const SUMMARY_CHARS: usize = 2000;
pub(crate) const REASON_CHARS: usize = 500;
pub(crate) const NAME_CHARS: usize = 200;

/// `text` without control characters (newlines and tabs kept), trimmed and
/// cut to `max` characters with an ellipsis; `None` if nothing is left.
pub(crate) fn bounded(text: &str, max: usize) -> Option<String> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    if clean.chars().count() <= max {
        return Some(clean.into());
    }
    let head: String = clean.chars().take(max.saturating_sub(1)).collect();
    Some(format!("{}…", head.trim_end()))
}

/// [`bounded`], on a single line: newlines and tabs become spaces.
pub(crate) fn bounded_line(text: &str, max: usize) -> Option<String> {
    let line: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect();
    bounded(&line, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cleaned_trimmed_and_cut() {
        assert_eq!(bounded("  \u{1b}[31mhi\u{7}\n ", 10), Some("[31mhi".into()));
        assert_eq!(bounded(" \n ", 10), None);
        assert_eq!(bounded("abcdefghij", 5), Some("abcd…".into()));
        assert_eq!(bounded_line("a\nb", 10), Some("a b".into()));
    }
}
