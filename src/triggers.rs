//! "Linked" tasks: offer to start saved tasks when their program or browser page comes to
//! the front. Pure logic here (rules, matching, cooldown); the window watcher calls it.

use crate::config::{ClickerConfig, ClickerKind};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Don't ask again about the same window for this long after asking (whatever the answer).
pub const COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// One prompt: every enabled task that links to the same program + title text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub ids: Vec<u64>,
    pub names: Vec<String>,
    /// Lower-case program file name.
    pub process: String,
    /// Lower-case title text ("" = any window of the program).
    pub contains: String,
    /// What to call the window in the prompt ("YouTube", "Notepad").
    pub label: String,
}

impl Rule {
    pub fn key(&self) -> String {
        format!("{}|{}", self.process, self.contains)
    }

    pub fn matches(&self, process: &str, title: &str) -> bool {
        process.eq_ignore_ascii_case(&self.process)
            && (self.contains.is_empty() || title.to_lowercase().contains(&self.contains))
    }
}

fn program_label(process: &str) -> String {
    let p = process.trim_end_matches(".exe").trim_end_matches(".EXE");
    let mut c = p.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Build the rules from the active profile's tasks. Classic tasks have no window, so they can
/// never be linked.
pub fn build_rules(clickers: &[ClickerConfig]) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    for c in clickers {
        if !c.trigger.enabled || c.kind() == ClickerKind::Classic || !c.target.is_set() {
            continue;
        }
        let process = if c.trigger.process.trim().is_empty() { &c.target.process } else { &c.trigger.process };
        let process = process.trim().to_lowercase();
        if process.is_empty() {
            continue;
        }
        let contains = c.trigger.title_contains.trim().to_lowercase();
        match rules.iter_mut().find(|r| r.process == process && r.contains == contains) {
            Some(r) => {
                r.ids.push(c.id);
                r.names.push(c.name.clone());
            }
            None => {
                let label = if contains.is_empty() {
                    program_label(&process)
                } else {
                    c.trigger.title_contains.trim().to_string()
                };
                rules.push(Rule { ids: vec![c.id], names: vec![c.name.clone()], process, contains, label });
            }
        }
    }
    rules
}

/// Remembers when we last asked about a window, so the user isn't asked again every time a
/// browser tab switches back.
#[derive(Default)]
pub struct Cooldowns {
    asked: HashMap<(isize, String), Instant>,
}

impl Cooldowns {
    pub fn should_ask(&mut self, hwnd: isize, rule: &Rule, now: Instant) -> bool {
        self.asked.retain(|_, t| now.duration_since(*t) < COOLDOWN);
        !self.asked.contains_key(&(hwnd, rule.key()))
    }

    pub fn mark(&mut self, hwnd: isize, rule: &Rule, now: Instant) {
        self.asked.insert((hwnd, rule.key()), now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{suggested_title_hint, ClickerConfig, Lang, TargetInfo};

    fn target(title: &str, process: &str) -> TargetInfo {
        TargetInfo { title: title.into(), class: "Chrome_WidgetWin_1".into(), process: process.into(), hwnd: None }
    }

    #[test]
    fn suggests_site_names_for_browsers_only() {
        assert_eq!(suggested_title_hint(&target("Watch later - YouTube - Google Chrome", "chrome.exe")), "YouTube");
        assert_eq!(suggested_title_hint(&target("Inbox (3) - me@x.com - Gmail - Google Chrome", "chrome.exe")), "Gmail");
        assert_eq!(suggested_title_hint(&target("Wikipedia \u{2014} Mozilla Firefox", "firefox.exe")), "Wikipedia");
        assert_eq!(suggested_title_hint(&target("Untitled - Notepad", "notepad.exe")), "");
    }

    fn linked(id: u64, kind: ClickerKind, process: &str, contains: &str) -> ClickerConfig {
        let mut c = ClickerConfig::new_of_kind(id, kind, id as usize, Lang::En);
        c.target = target("x", process);
        c.trigger.enabled = true;
        c.trigger.title_contains = contains.into();
        c
    }

    #[test]
    fn groups_tasks_and_skips_classic_and_disabled() {
        let mut off = linked(4, ClickerKind::Simple, "chrome.exe", "YouTube");
        off.trigger.enabled = false;
        let tasks = vec![
            linked(1, ClickerKind::Simple, "chrome.exe", "YouTube"),
            linked(2, ClickerKind::Keyboard, "CHROME.EXE", "youtube"),
            linked(3, ClickerKind::Classic, "chrome.exe", "YouTube"),
            off,
            linked(5, ClickerKind::Advanced, "notepad.exe", ""),
        ];
        let rules = build_rules(&tasks);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].ids, vec![1, 2], "same program + text share one prompt");
        assert_eq!(rules[0].label, "YouTube");
        assert_eq!(rules[1].label, "Notepad");
        assert!(rules[0].matches("chrome.exe", "Cat videos - YouTube - Google Chrome"));
        assert!(!rules[0].matches("chrome.exe", "Gmail - Google Chrome"));
        assert!(!rules[0].matches("msedge.exe", "YouTube - Microsoft Edge"));
        assert!(rules[1].matches("NOTEPAD.EXE", "anything"));
    }

    #[test]
    fn asks_once_per_window_within_cooldown() {
        let r = build_rules(&[linked(1, ClickerKind::Simple, "chrome.exe", "YouTube")]).remove(0);
        let mut cd = Cooldowns::default();
        let t0 = Instant::now();
        assert!(cd.should_ask(10, &r, t0));
        cd.mark(10, &r, t0);
        assert!(!cd.should_ask(10, &r, t0 + Duration::from_secs(60)), "same window: not again soon");
        assert!(cd.should_ask(11, &r, t0 + Duration::from_secs(60)), "another window: ask");
        assert!(cd.should_ask(10, &r, t0 + COOLDOWN + Duration::from_secs(1)), "after the cooldown: ask again");
    }
}
