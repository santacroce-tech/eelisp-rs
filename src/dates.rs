//! Date/time helpers for the date builtins (now/today/date-format/date-add/date-diff).
//! String dates are "YYYY-MM-DD"; epoch times are f64 seconds since the Unix epoch (UTC).
//! Uses Howard Hinnant's civil algorithms — no external date crate.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn fmt_ymd(y: i64, m: i64, d: i64) -> String {
    format!("{:04}-{:02}-{:02}", y, m, d)
}

pub fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let head: String = s.chars().take(10).collect();
    let parts: Vec<&str> = head.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

pub fn now_epoch() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Midnight (UTC) of the current day, in epoch seconds.
pub fn today_epoch() -> f64 {
    let secs = now_epoch() as i64;
    (secs.div_euclid(86400) * 86400) as f64
}

pub struct DateTime {
    pub y: i64,
    pub mo: i64,
    pub d: i64,
    pub h: i64,
    pub mi: i64,
    pub s: i64,
}

pub fn from_epoch(secs: f64) -> DateTime {
    let secs = secs as i64;
    let (y, mo, d) = civil_from_days(secs.div_euclid(86400));
    let sod = secs.rem_euclid(86400);
    DateTime { y, mo, d, h: sod / 3600, mi: (sod % 3600) / 60, s: sod % 60 }
}

/// Parse "YYYY-MM-DD", "YYYY-MM-DDTHH:MM[:SS]" or "YYYY-MM-DD HH:MM[:SS]".
pub fn from_str(s: &str) -> Option<DateTime> {
    let (y, mo, d) = parse_ymd(s)?;
    let mut dt = DateTime { y, mo, d, h: 0, mi: 0, s: 0 };
    let rest: String = s.chars().skip(10).collect();
    let time = rest.trim_start_matches(['T', ' ']);
    if !time.is_empty() {
        let hms: Vec<&str> = time.split(['+', 'Z', '.']).next().unwrap_or("").split(':').collect();
        if let Some(h) = hms.first().and_then(|x| x.parse().ok()) {
            dt.h = h;
        }
        if let Some(m) = hms.get(1).and_then(|x| x.parse().ok()) {
            dt.mi = m;
        }
        if let Some(sec) = hms.get(2).and_then(|x| x.parse().ok()) {
            dt.s = sec;
        }
    }
    Some(dt)
}

const WEEKDAY_FULL: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAY_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_FULL: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTH_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// 0 = Sunday … 6 = Saturday.
fn weekday_index(y: i64, mo: i64, d: i64) -> usize {
    // epoch day 0 (1970-01-01) was a Thursday (index 4)
    ((days_from_civil(y, mo, d).rem_euclid(7) + 4).rem_euclid(7)) as usize
}

/// Format a DateTime with a subset of Unicode date patterns (yyyy MM dd HH mm ss EEEE EEE c a …).
pub fn format(pattern: &str, dt: &DateTime) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // quoted literal; '' → a literal quote
            i += 1;
            if i < chars.len() && chars[i] == '\'' {
                out.push('\'');
                i += 1;
                continue;
            }
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1; // skip closing quote
            continue;
        }
        if c.is_ascii_alphabetic() {
            let mut n = 1;
            while i + n < chars.len() && chars[i + n] == c {
                n += 1;
            }
            out.push_str(&token(c, n, dt));
            i += n;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn token(c: char, n: usize, dt: &DateTime) -> String {
    let wd = weekday_index(dt.y, dt.mo, dt.d);
    match c {
        'y' | 'Y' => {
            if n == 2 {
                format!("{:02}", dt.y.rem_euclid(100))
            } else {
                format!("{:0width$}", dt.y, width = n.max(1))
            }
        }
        'M' | 'L' => match n {
            1 => dt.mo.to_string(),
            2 => format!("{:02}", dt.mo),
            3 => MONTH_SHORT.get((dt.mo - 1) as usize).copied().unwrap_or("").to_string(),
            _ => MONTH_FULL.get((dt.mo - 1) as usize).copied().unwrap_or("").to_string(),
        },
        'd' => {
            if n >= 2 {
                format!("{:02}", dt.d)
            } else {
                dt.d.to_string()
            }
        }
        'H' => {
            if n >= 2 {
                format!("{:02}", dt.h)
            } else {
                dt.h.to_string()
            }
        }
        'h' => {
            let h12 = ((dt.h + 11) % 12) + 1;
            if n >= 2 {
                format!("{:02}", h12)
            } else {
                h12.to_string()
            }
        }
        'm' => {
            if n >= 2 {
                format!("{:02}", dt.mi)
            } else {
                dt.mi.to_string()
            }
        }
        's' => {
            if n >= 2 {
                format!("{:02}", dt.s)
            } else {
                dt.s.to_string()
            }
        }
        'E' => {
            if n >= 4 {
                WEEKDAY_FULL[wd].to_string()
            } else {
                WEEKDAY_SHORT[wd].to_string()
            }
        }
        'c' | 'e' => match n {
            4.. => WEEKDAY_FULL[wd].to_string(),
            3 => WEEKDAY_SHORT[wd].to_string(),
            // numeric ISO weekday 1=Mon … 7=Sun
            _ => (((wd + 6) % 7) + 1).to_string(),
        },
        'a' => {
            if dt.h < 12 {
                "AM".into()
            } else {
                "PM".into()
            }
        }
        _ => c.to_string().repeat(n),
    }
}

/// (date-add "YYYY-MM-DD" amount unit) → "YYYY-MM-DD". unit = days|weeks|months.
pub fn date_add(date: &str, amount: i64, unit: &str) -> Option<String> {
    let (y, m, d) = parse_ymd(date)?;
    match unit {
        "months" => {
            let total = (m - 1) + amount;
            let ny = y + total.div_euclid(12);
            let nm = total.rem_euclid(12) + 1;
            let nd = d.min(days_in_month(ny, nm));
            Some(fmt_ymd(ny, nm, nd))
        }
        "weeks" => {
            let (ny, nm, nd) = civil_from_days(days_from_civil(y, m, d) + amount * 7);
            Some(fmt_ymd(ny, nm, nd))
        }
        _ => {
            let (ny, nm, nd) = civil_from_days(days_from_civil(y, m, d) + amount);
            Some(fmt_ymd(ny, nm, nd))
        }
    }
}

/// (date-diff d1 d2) → whole days from d2 to d1 (i.e. d1 − d2).
pub fn date_diff(d1: &str, d2: &str) -> Option<i64> {
    let (y1, m1, dd1) = parse_ymd(d1)?;
    let (y2, m2, dd2) = parse_ymd(d2)?;
    Some(days_from_civil(y1, m1, dd1) - days_from_civil(y2, m2, dd2))
}
