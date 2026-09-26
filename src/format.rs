//! Turns ai-usagebar entries into the short strings herdr shows in sidebar
//! rows. The sidebar is 26 columns wide by default, so everything here aims
//! for compact text such as `◔ 5h 0% · 7d 13%`.

use crate::redact::redact;
use crate::usage::{Entry, Metric};

/// herdr caps reported token values at 80 characters.
pub const MAX_TOKEN_CHARS: usize = 80;
const MAX_ERROR_CHARS: usize = 24;
const MAX_LABEL_CHARS: usize = 7;

pub const WARNING: char = '⚠';
pub const UNAVAILABLE: &str = "⚠ usage unavailable";
pub const MISSING_BINARY: &str = "⚠ ai-usagebar not found";

/// A pie that fills with usage. The glyph doubles as the hook for sidebar
/// color rules (`starts_with`), since herdr can only style a token by its own
/// value.
pub fn pie(percent: u32) -> char {
    match percent {
        0..=12 => '○',
        13..=37 => '◔',
        38..=62 => '◑',
        63..=87 => '◕',
        _ => '●',
    }
}

/// Agent-row text for one provider, e.g. `◔ 5h 0% · 7d 13%`.
pub fn entry_line(entry: &Entry, max_windows: usize) -> Option<String> {
    let windows: Vec<String> = entry
        .headline_metrics()
        .take(max_windows)
        .map(|metric| {
            let label = window_label(metric);
            let value = metric_value(metric);
            if label.is_empty() {
                value
            } else {
                format!("{label} {value}")
            }
        })
        .collect();

    if windows.is_empty() {
        return entry
            .error
            .as_deref()
            .map(|error| cap(&format!("{WARNING} {}", short_error(error))));
    }

    let glyph = pie(entry.max_percent().unwrap_or(0));
    let stale = if entry.stale { " (stale)" } else { "" };
    Some(cap(&format!("{glyph} {}{stale}", windows.join(" · "))))
}

/// Workspace-row text across several providers, e.g. `◔ cld 13% · gpt 0%`.
pub fn summary_line<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Option<String> {
    let mut parts = Vec::new();
    let mut worst: Option<u32> = None;
    let mut stale = false;
    for entry in entries {
        match entry.max_percent() {
            Some(percent) => {
                worst = worst.max(Some(percent));
                stale |= entry.stale;
                parts.push(format!("{} {percent}%", entry.short_name()));
            }
            None if entry.error.is_some() => {
                parts.push(format!("{} {WARNING}", entry.short_name()));
            }
            None => {}
        }
    }
    if parts.is_empty() {
        return None;
    }
    let glyph = worst.map_or(WARNING, pie);
    let stale = if stale { " (stale)" } else { "" };
    Some(cap(&format!("{glyph} {}{stale}", parts.join(" · "))))
}

pub fn window_label(metric: &Metric) -> String {
    if let Some(secs) = metric.window_secs.filter(|secs| *secs > 0) {
        return compact_duration(secs);
    }
    let label = metric.label.to_lowercase();
    if label.contains("month") {
        "mo".into()
    } else if label.contains("week") {
        "wk".into()
    } else if label.contains("day") || label.contains("daily") {
        "day".into()
    } else {
        label
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .chars()
            .take(MAX_LABEL_CHARS)
            .collect()
    }
}

pub fn metric_value(metric: &Metric) -> String {
    let value = metric.value.trim();
    if value.eq_ignore_ascii_case("unlimited") {
        return "∞".into();
    }
    match (metric.headline.as_str(), metric.percent) {
        ("value", _) if !value.is_empty() => value.into(),
        (_, Some(percent)) if percent.is_finite() => format!("{}%", percent.round()),
        _ => value.into(),
    }
}

pub fn compact_duration(secs: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if secs.is_multiple_of(DAY) {
        format!("{}d", secs / DAY)
    } else if secs.is_multiple_of(HOUR) {
        format!("{}h", secs / HOUR)
    } else if secs.is_multiple_of(MINUTE) {
        format!("{}m", secs / MINUTE)
    } else {
        format!("{secs}s")
    }
}

/// ai-usagebar errors are full sentences (`credentials error: Zai: no API
/// key. Either set ...`); keep the leading category that fits a sidebar row.
pub fn short_error(error: &str) -> String {
    if error.to_lowercase().contains("rate limit") {
        return "rate limited".into();
    }
    let error = redact(error);
    let head = error
        .split([':', ';', '.'])
        .next()
        .unwrap_or_default()
        .trim();
    let head = if head.is_empty() { "error" } else { head };
    truncate(head, MAX_ERROR_CHARS)
}

fn cap(text: &str) -> String {
    truncate(text, MAX_TOKEN_CHARS)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Report;

    fn sample() -> Report {
        Report::parse(include_str!("../tests/fixtures/usage.json")).unwrap()
    }

    fn entry(report: &Report, id: &str) -> Entry {
        report.entries.iter().find(|e| e.id == id).unwrap().clone()
    }

    fn percent_metric(label: &str, percent: f64, window_secs: Option<u64>) -> Metric {
        Metric {
            label: label.into(),
            percent: Some(percent),
            value: format!("{percent}%"),
            headline: "percent".into(),
            window_secs,
            group: None,
        }
    }

    #[test]
    fn pie_fills_with_usage() {
        assert_eq!(pie(0), '○');
        assert_eq!(pie(12), '○');
        assert_eq!(pie(13), '◔');
        assert_eq!(pie(50), '◑');
        assert_eq!(pie(75), '◕');
        assert_eq!(pie(88), '●');
        assert_eq!(pie(100), '●');
        assert_eq!(pie(250), '●');
    }

    #[test]
    fn claude_line_shows_the_first_two_windows() {
        let report = sample();
        assert_eq!(
            entry_line(&entry(&report, "anthropic"), 2).unwrap(),
            "◔ 5h 0% · 7d 13%"
        );
    }

    #[test]
    fn max_windows_limits_the_line() {
        let report = sample();
        assert_eq!(
            entry_line(&entry(&report, "anthropic"), 1).unwrap(),
            "◔ 5h 0%"
        );
        assert_eq!(
            entry_line(&entry(&report, "anthropic"), 3).unwrap(),
            "◔ 5h 0% · 7d 13% · 7d 4%"
        );
    }

    #[test]
    fn glyph_reflects_a_hidden_window_near_its_limit() {
        let claude = Entry {
            id: "anthropic".into(),
            metrics: vec![
                percent_metric("Session (5h)", 10.0, Some(18_000)),
                percent_metric("Weekly (7d)", 20.0, Some(604_800)),
                percent_metric("Fable (7d)", 97.0, Some(604_800)),
            ],
            ..Entry::default()
        };
        assert_eq!(entry_line(&claude, 2).unwrap(), "● 5h 10% · 7d 20%");
    }

    #[test]
    fn unlimited_copilot_quota_is_shown_as_infinity() {
        let report = sample();
        assert_eq!(
            entry_line(&entry(&report, "copilot"), 2).unwrap(),
            "○ premium ∞ · chat ∞"
        );
    }

    #[test]
    fn error_entry_shows_its_error_category() {
        let report = sample();
        assert_eq!(
            entry_line(&entry(&report, "zai"), 2).unwrap(),
            "⚠ credentials error"
        );
    }

    #[test]
    fn stale_entries_are_marked() {
        let report = sample().into_stale();
        assert_eq!(
            entry_line(&entry(&report, "openai"), 2).unwrap(),
            "○ 5h 0% · 7d 0% (stale)"
        );
    }

    #[test]
    fn ready_entry_without_metrics_has_no_line() {
        let empty = Entry {
            id: "x".into(),
            ..Entry::default()
        };
        assert_eq!(entry_line(&empty, 2), None);
    }

    #[test]
    fn value_headline_shows_the_value() {
        let balance = Metric {
            label: "Balance".into(),
            percent: Some(40.0),
            value: "$4.20".into(),
            headline: "value".into(),
            ..Metric::default()
        };
        assert_eq!(metric_value(&balance), "$4.20");
        assert_eq!(window_label(&balance), "balance");
    }

    #[test]
    fn window_label_falls_back_to_the_label_text() {
        let monthly = percent_metric("Monthly quota", 5.0, None);
        let weekly = percent_metric("Weekly", 5.0, None);
        let daily = percent_metric("Daily requests", 5.0, None);
        let other = percent_metric("Completions per model", 5.0, None);
        assert_eq!(window_label(&monthly), "mo");
        assert_eq!(window_label(&weekly), "wk");
        assert_eq!(window_label(&daily), "day");
        assert_eq!(window_label(&other), "complet");
    }

    #[test]
    fn compact_duration_picks_the_largest_whole_unit() {
        assert_eq!(compact_duration(18_000), "5h");
        assert_eq!(compact_duration(604_800), "7d");
        assert_eq!(compact_duration(2_592_000), "30d");
        assert_eq!(compact_duration(5_400), "90m");
        assert_eq!(compact_duration(61), "61s");
    }

    #[test]
    fn short_error_keeps_the_category() {
        assert_eq!(
            short_error("credentials error: Zai: no API key."),
            "credentials error"
        );
        assert_eq!(
            short_error("rate limited; next attempt in 4m"),
            "rate limited"
        );
        assert_eq!(
            short_error("HTTP 429: Too Many Requests (rate limit)"),
            "rate limited"
        );
        assert_eq!(
            short_error("an extraordinarily long error category without separators"),
            "an extraordinarily long…"
        );
        assert_eq!(short_error(""), "error");
        assert_eq!(
            short_error("sk-ant-abc123 was rejected"),
            "[redacted] was rejected"
        );
    }

    #[test]
    fn summary_line_lists_each_provider() {
        let report = sample();
        let summary = summary_line(&report.entries).unwrap();
        assert_eq!(summary, "◔ cld 13% · gpt 0% · ghc 0% · zai ⚠");
    }

    #[test]
    fn summary_of_only_errors_uses_the_warning_glyph() {
        let report = sample();
        let zai = entry(&report, "zai");
        assert_eq!(summary_line([&zai]).unwrap(), "⚠ zai ⚠");
    }

    #[test]
    fn summary_of_nothing_is_none() {
        assert_eq!(summary_line(std::iter::empty()), None);
    }

    #[test]
    fn long_lines_are_capped_at_the_herdr_limit() {
        let many = Entry {
            id: "custom:x".into(),
            metrics: (0..20)
                .map(|i| percent_metric(&format!("Window{i}"), 1.0, None))
                .collect(),
            ..Entry::default()
        };
        let line = entry_line(&many, 20).unwrap();
        assert_eq!(line.chars().count(), MAX_TOKEN_CHARS);
        assert!(line.ends_with('…'));
    }
}
