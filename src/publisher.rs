//! Pushes a [`Plan`] to herdr with as few calls as possible: a target is
//! re-reported only when its tokens changed or its TTL needs refreshing, and
//! targets that dropped out of the plan are cleared.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::log;
use crate::plan::{Plan, TOKENS, Target, Tokens};

pub trait Sink {
    /// Set `tokens` on `target` and clear every name in `clear`, expiring
    /// after `ttl`.
    fn report(
        &mut self,
        target: &Target,
        tokens: &Tokens,
        clear: &[&str],
        ttl: Duration,
    ) -> Result<()>;
}

pub struct Publisher {
    ttl: Duration,
    published: HashMap<Target, (Tokens, Instant)>,
}

impl Publisher {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            published: HashMap::new(),
        }
    }

    /// Re-report unchanged tokens once a third of the TTL has passed, so a
    /// missed tick or two never lets live values expire.
    fn refresh_after(&self) -> Duration {
        self.ttl / 3
    }

    pub fn apply(&mut self, plan: Plan, now: Instant, sink: &mut impl Sink) {
        let stale_targets: Vec<Target> = self
            .published
            .keys()
            .filter(|target| !plan.contains_key(*target))
            .cloned()
            .collect();
        for target in stale_targets {
            self.published.remove(&target);
            // The pane or workspace may already be gone; TTL expiry is the
            // backstop, so a failed clear is only worth a log line.
            if let Err(error) = sink.report(&target, &Tokens::new(), &TOKENS, self.ttl) {
                log::debug(&format!("clear {target:?}: {error:#}"));
            }
        }

        for (target, tokens) in plan {
            let fresh = self.published.get(&target).is_some_and(|(previous, at)| {
                *previous == tokens && now.duration_since(*at) < self.refresh_after()
            });
            if fresh {
                continue;
            }
            let clear: Vec<&str> = TOKENS
                .into_iter()
                .filter(|name| !tokens.contains_key(name))
                .collect();
            match sink.report(&target, &tokens, &clear, self.ttl) {
                Ok(()) => {
                    self.published.insert(target, (tokens, now));
                }
                Err(error) => {
                    self.published.remove(&target);
                    log::info(&format!("report {target:?}: {error:#}"));
                }
            }
        }
    }

    pub fn clear_all(&mut self, sink: &mut impl Sink) {
        self.apply(Plan::new(), Instant::now(), sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{TOKEN_PERCENT, TOKEN_USAGE};

    #[derive(Default)]
    struct RecordingSink {
        calls: Vec<(Target, Tokens, Vec<String>)>,
        fail: bool,
    }

    impl Sink for RecordingSink {
        fn report(
            &mut self,
            target: &Target,
            tokens: &Tokens,
            clear: &[&str],
            _ttl: Duration,
        ) -> Result<()> {
            self.calls.push((
                target.clone(),
                tokens.clone(),
                clear.iter().map(|name| name.to_string()).collect(),
            ));
            if self.fail {
                anyhow::bail!("pane not found");
            }
            Ok(())
        }
    }

    const TTL: Duration = Duration::from_secs(180);

    fn pane(id: &str) -> Target {
        Target::Pane(id.into())
    }

    fn plan_with(target: Target, usage: &str, percent: Option<&str>) -> Plan {
        let mut tokens = Tokens::from([(TOKEN_USAGE, usage.to_string())]);
        if let Some(percent) = percent {
            tokens.insert(TOKEN_PERCENT, percent.to_string());
        }
        Plan::from([(target, tokens)])
    }

    #[test]
    fn first_apply_reports_and_clears_unused_tokens() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink::default();
        publisher.apply(
            plan_with(pane("p1"), "◔ 5h 13%", None),
            Instant::now(),
            &mut sink,
        );

        assert_eq!(sink.calls.len(), 1);
        let (target, tokens, clear) = &sink.calls[0];
        assert_eq!(*target, pane("p1"));
        assert_eq!(tokens[TOKEN_USAGE], "◔ 5h 13%");
        assert_eq!(clear, &[TOKEN_PERCENT.to_string()]);
    }

    #[test]
    fn unchanged_tokens_are_not_re_reported_until_the_refresh_point() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink::default();
        let start = Instant::now();
        publisher.apply(plan_with(pane("p1"), "a", Some("1")), start, &mut sink);
        publisher.apply(
            plan_with(pane("p1"), "a", Some("1")),
            start + Duration::from_secs(30),
            &mut sink,
        );
        assert_eq!(sink.calls.len(), 1);

        publisher.apply(
            plan_with(pane("p1"), "a", Some("1")),
            start + TTL / 3,
            &mut sink,
        );
        assert_eq!(sink.calls.len(), 2);
    }

    #[test]
    fn changed_tokens_are_reported_immediately() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink::default();
        let start = Instant::now();
        publisher.apply(plan_with(pane("p1"), "a", None), start, &mut sink);
        publisher.apply(plan_with(pane("p1"), "b", None), start, &mut sink);
        assert_eq!(sink.calls.len(), 2);
        assert_eq!(sink.calls[1].1[TOKEN_USAGE], "b");
    }

    #[test]
    fn targets_leaving_the_plan_are_cleared_once() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink::default();
        let start = Instant::now();
        publisher.apply(plan_with(pane("p1"), "a", Some("1")), start, &mut sink);
        publisher.apply(Plan::new(), start, &mut sink);
        publisher.apply(Plan::new(), start, &mut sink);

        assert_eq!(sink.calls.len(), 2);
        let (target, tokens, clear) = &sink.calls[1];
        assert_eq!(*target, pane("p1"));
        assert!(tokens.is_empty());
        assert_eq!(clear, &[TOKEN_USAGE.to_string(), TOKEN_PERCENT.to_string()]);
    }

    #[test]
    fn a_failed_report_is_retried_on_the_next_apply() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink {
            fail: true,
            ..RecordingSink::default()
        };
        let start = Instant::now();
        publisher.apply(plan_with(pane("p1"), "a", None), start, &mut sink);
        sink.fail = false;
        publisher.apply(plan_with(pane("p1"), "a", None), start, &mut sink);
        publisher.apply(plan_with(pane("p1"), "a", None), start, &mut sink);
        assert_eq!(sink.calls.len(), 2);
    }

    #[test]
    fn clear_all_clears_everything_published() {
        let mut publisher = Publisher::new(TTL);
        let mut sink = RecordingSink::default();
        let mut plan = plan_with(pane("p1"), "a", None);
        plan.extend(plan_with(Target::Workspace("w1".into()), "b", None));
        publisher.apply(plan, Instant::now(), &mut sink);
        publisher.clear_all(&mut sink);

        let cleared: Vec<&Target> = sink.calls[2..].iter().map(|(target, ..)| target).collect();
        assert_eq!(cleared.len(), 2);
        assert!(cleared.contains(&&pane("p1")));
        assert!(cleared.contains(&&Target::Workspace("w1".into())));
    }
}
