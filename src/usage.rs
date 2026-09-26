//! Model of the `ai-usagebar usage --json` document.
//!
//! ai-usagebar calls this a tolerant contract: fields may be added or omitted
//! without a version bump, so every field here defaults and unknown fields are
//! ignored. Only `schema_version` is checked.

use anyhow::{Result, bail};
use serde::Deserialize;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Report {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Metric {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub percent: Option<f64>,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub headline: String,
    #[serde(default)]
    pub window_secs: Option<u64>,
    #[serde(default)]
    pub group: Option<String>,
}

impl Report {
    pub fn parse(json: &str) -> Result<Self> {
        let report: Report = serde_json::from_str(json)?;
        if report.schema_version != SUPPORTED_SCHEMA_VERSION {
            bail!(
                "unsupported ai-usagebar usage schema_version {} (expected {})",
                report.schema_version,
                SUPPORTED_SCHEMA_VERSION
            );
        }
        Ok(report)
    }

    /// Mark every entry stale, used when a refresh fails and the previous
    /// report is still being shown.
    pub fn into_stale(mut self) -> Self {
        for entry in &mut self.entries {
            entry.stale = true;
        }
        self
    }
}

impl Entry {
    pub fn is_ready(&self) -> bool {
        self.error.is_none() && self.status != "error"
    }

    /// Top-level gauges, excluding sub-group rows such as per-product
    /// breakdowns that only make sense beneath their heading.
    pub fn headline_metrics(&self) -> impl Iterator<Item = &Metric> {
        self.metrics.iter().filter(|metric| metric.group.is_none())
    }

    /// Highest usage across every top-level gauge, so a nearly exhausted
    /// window is flagged even when it is not one of the windows displayed.
    pub fn max_percent(&self) -> Option<u32> {
        self.headline_metrics()
            .filter_map(|metric| metric.percent)
            .filter(|percent| percent.is_finite())
            .map(|percent| percent.clamp(0.0, 999.0).round() as u32)
            .max()
    }

    pub fn short_name(&self) -> &str {
        if !self.short_name.is_empty() {
            &self.short_name
        } else {
            &self.id
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/usage.json");

    #[test]
    fn parses_the_sample_report() {
        let report = Report::parse(SAMPLE).unwrap();
        let ids: Vec<&str> = report.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["anthropic", "openai", "copilot", "zai"]);

        let claude = &report.entries[0];
        assert_eq!(claude.short_name, "cld");
        assert!(claude.is_ready());
        assert_eq!(claude.metrics[0].window_secs, Some(18_000));
        assert_eq!(claude.metrics[1].percent, Some(13.0));
    }

    #[test]
    fn error_entries_are_not_ready() {
        let report = Report::parse(SAMPLE).unwrap();
        let zai = report.entries.iter().find(|e| e.id == "zai").unwrap();
        assert!(!zai.is_ready());
        assert!(zai.metrics.is_empty());
        assert_eq!(zai.max_percent(), None);
    }

    #[test]
    fn rejects_an_unknown_schema_version() {
        let err = Report::parse(r#"{"schema_version": 2, "entries": []}"#).unwrap_err();
        assert!(err.to_string().contains("schema_version 2"));
    }

    #[test]
    fn rejects_a_document_without_schema_version() {
        assert!(Report::parse(r#"{"entries": []}"#).is_err());
    }

    #[test]
    fn tolerates_unknown_and_missing_fields() {
        let json = r#"{"schema_version":1,"future":true,"entries":[{"id":"x","new_field":[1]}]}"#;
        let report = Report::parse(json).unwrap();
        assert_eq!(report.entries[0].id, "x");
        assert_eq!(report.entries[0].short_name(), "x");
        assert!(report.entries[0].is_ready());
    }

    #[test]
    fn max_percent_ignores_grouped_rows_and_takes_the_worst_window() {
        let entry = Entry {
            id: "grok".into(),
            metrics: vec![
                Metric {
                    percent: Some(20.0),
                    ..Metric::default()
                },
                Metric {
                    percent: Some(91.4),
                    ..Metric::default()
                },
                Metric {
                    percent: Some(100.0),
                    group: Some("Breakdown".into()),
                    ..Metric::default()
                },
            ],
            ..Entry::default()
        };
        assert_eq!(entry.max_percent(), Some(91));
    }

    #[test]
    fn into_stale_marks_every_entry() {
        let report = Report::parse(SAMPLE).unwrap().into_stale();
        assert!(report.entries.iter().all(|entry| entry.stale));
    }
}
