//! Regex NLP for `(add ...)` / `(smart-parse ...)` (ANALYSIS §4.8).
//! Extracts a date, priority, and people from free text, returning the cleaned remainder.
//!
//! Dates resolve relative to a caller-supplied `today` (so the core stays clock-free/testable).

use regex::Regex;

use crate::agenda::add_days;

pub struct SmartResult {
    pub text: String,
    pub when: Option<String>,
    pub priority: Option<i64>,
    pub who: Vec<String>,
}

fn strip(text: &str, pat: &str) -> String {
    Regex::new(pat).unwrap().replace_all(text, " ").to_string()
}

pub fn parse(input: &str, today: &str) -> SmartResult {
    let mut text = input.to_string();
    let mut when = None;
    let mut priority = None;
    let mut who: Vec<String> = Vec::new();

    // ── date (first match wins) ──
    if let Some(c) = Regex::new(r"\b(\d{4}-\d{2}-\d{2})\b").unwrap().captures(&text) {
        when = Some(c[1].to_string());
        text = text.replacen(&c[0], " ", 1);
    } else if Regex::new(r"(?i)\btomorrow\b").unwrap().is_match(&text) {
        when = add_days(today, 1);
        text = strip(&text, r"(?i)\btomorrow\b");
    } else if Regex::new(r"(?i)\byesterday\b").unwrap().is_match(&text) {
        when = add_days(today, -1);
        text = strip(&text, r"(?i)\byesterday\b");
    } else if Regex::new(r"(?i)\btoday\b").unwrap().is_match(&text) {
        when = Some(today.to_string());
        text = strip(&text, r"(?i)\btoday\b");
    } else if let Some(c) = Regex::new(r"(?i)\bin (\d+) days?\b").unwrap().captures(&text) {
        when = add_days(today, c[1].parse().unwrap_or(0));
        text = text.replacen(&c[0], " ", 1);
    } else if let Some(c) = Regex::new(r"(?i)\bin (\d+) weeks?\b").unwrap().captures(&text) {
        when = add_days(today, c[1].parse::<i64>().unwrap_or(0) * 7);
        text = text.replacen(&c[0], " ", 1);
    }

    // ── priority ──
    if Regex::new(r"(?i)\b(urgent|asap)\b").unwrap().is_match(&text) {
        priority = Some(1);
        text = strip(&text, r"(?i)\b(urgent|asap)\b");
    } else if Regex::new(r"(?i)\bhigh priority\b").unwrap().is_match(&text) {
        priority = Some(2);
        text = strip(&text, r"(?i)\bhigh priority\b");
    } else if Regex::new(r"(?i)\blow priority\b").unwrap().is_match(&text) {
        priority = Some(4);
        text = strip(&text, r"(?i)\blow priority\b");
    }
    if let Some(c) = Regex::new(r"(?:^|\s)(!{1,3})(?:\s|$)").unwrap().captures(&text) {
        if priority.is_none() {
            priority = Some(match c[1].len() {
                3 => 1,
                2 => 2,
                _ => 3,
            });
        }
        let bangs = c[1].to_string();
        text = text.replacen(&bangs, " ", 1);
    }

    // ── people ──
    let at = Regex::new(r"@([A-Za-z]\w*)").unwrap();
    for c in at.captures_iter(&text) {
        let name = c[1].to_string();
        if !who.contains(&name) {
            who.push(name);
        }
    }
    text = strip(&text, r"@[A-Za-z]\w*");

    let verb = Regex::new(r"(?i)\b(?:call|email|meet|text|with|for|from)\s+([A-Z][a-z]+)\b").unwrap();
    let scan = text.clone();
    for c in verb.captures_iter(&scan) {
        let name = c[1].to_string();
        if !is_common_word(&name) && !who.contains(&name) {
            who.push(name);
        }
    }

    let text = Regex::new(r"\s{2,}").unwrap().replace_all(text.trim(), " ").to_string();
    SmartResult { text, when, priority, who }
}

fn is_common_word(w: &str) -> bool {
    matches!(
        w,
        "The" | "This" | "That" | "These" | "Those" | "Some" | "Any" | "All" | "New" | "Next"
            | "Last" | "First" | "Monday" | "Tuesday" | "Wednesday" | "Thursday" | "Friday"
            | "Saturday" | "Sunday" | "January" | "February" | "March" | "April" | "May" | "June"
            | "July" | "August" | "September" | "October" | "November" | "December"
    )
}
