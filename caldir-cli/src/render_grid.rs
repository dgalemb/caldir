use caldir_core::event::{Event, EventTime};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Timelike, Weekday};
use owo_colors::OwoColorize;

use crate::utils::date::zoned_to_local;

pub struct GridEvent<'a> {
    pub event: &'a Event,
    pub color: (u8, u8, u8),
}

const FALLBACK_COLORS: &[(u8, u8, u8)] = &[
    (66, 133, 244),
    (52, 168, 83),
    (251, 188, 4),
    (234, 67, 53),
    (171, 71, 188),
    (255, 145, 0),
    (0, 188, 212),
];

pub fn color_for(slug: &str, configured: Option<&str>) -> (u8, u8, u8) {
    if let Some(hex) = configured.and_then(parse_hex_color) {
        return hex;
    }
    let h = slug.bytes().fold(0u32, |a, b| a.wrapping_add(b as u32));
    FALLBACK_COLORS[(h as usize) % FALLBACK_COLORS.len()]
}

fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Render events spanning [from, to] as one or more week-grid views.
pub fn render(events: &[GridEvent], from: NaiveDate, to: NaiveDate) {
    let term_width = terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100);

    let hour_col = 4;
    let day_col = ((term_width.saturating_sub(hour_col)) / 7).max(8);

    let mut week = monday_of(from);
    let mut first = true;
    let mut last_label: Option<String> = None;
    while week <= to {
        if !first {
            println!();
        }
        first = false;
        let label = month_label(week, week + Duration::days(6));
        if last_label.as_ref() != Some(&label) {
            println!("{}", label.bold());
            last_label = Some(label);
        }
        render_week(events, week, hour_col, day_col);
        week += Duration::days(7);
    }
}

fn render_week(events: &[GridEvent], week_start: NaiveDate, hour_col: usize, day_col: usize) {
    let week_end = week_start + Duration::days(6);

    let mut all_day: Vec<Vec<&GridEvent>> = (0..7).map(|_| Vec::new()).collect();
    let mut timed: Vec<Vec<TimedSlot>> = (0..7).map(|_| Vec::new()).collect();
    let mut min_h: Option<u32> = None;
    let mut max_h: Option<u32> = None;

    for ge in events {
        let date = local_date(&ge.event.start);
        if date < week_start || date > week_end {
            continue;
        }
        let day_idx = (date - week_start).num_days() as usize;

        match local_dt(&ge.event.start) {
            None => all_day[day_idx].push(ge),
            Some(start_dt) => {
                let end_dt = local_dt(&ge.event.end).unwrap_or(start_dt + Duration::hours(1));
                let sh = start_dt.hour();
                let eh_raw = end_dt.hour() + if end_dt.minute() > 0 { 1 } else { 0 };
                let eh = eh_raw.max(sh + 1).min(24);
                min_h = Some(min_h.map_or(sh, |m| m.min(sh)));
                max_h = Some(max_h.map_or(eh, |m| m.max(eh)));
                timed[day_idx].push(TimedSlot {
                    ge,
                    start: start_dt,
                    end: end_dt,
                });
            }
        }
    }

    print_header(week_start, hour_col, day_col);

    if all_day.iter().any(|v| !v.is_empty()) {
        print_all_day(&all_day, hour_col, day_col);
    }

    let (Some(mn), Some(mx)) = (min_h, max_h) else {
        if all_day.iter().all(|v| v.is_empty()) {
            println!("{}", "  no events this week".dimmed());
        }
        return;
    };

    for h in mn..mx {
        print_hour_row(h, &timed, hour_col, day_col);
    }
}

fn month_label(start: NaiveDate, end: NaiveDate) -> String {
    if start.month() == end.month() {
        start.format("%B %Y").to_string()
    } else {
        format!("{} – {}", start.format("%b"), end.format("%b %Y"))
    }
}

fn print_header(week_start: NaiveDate, hour_col: usize, day_col: usize) {
    let mut row = " ".repeat(hour_col);
    for i in 0..7 {
        let d = week_start + Duration::days(i);
        let label = format!("{} {}", weekday_short(d.weekday()), d.day());
        row.push_str(&pad(&label, day_col));
    }
    println!("{}", row.bold());
}

fn print_all_day(all_day: &[Vec<&GridEvent>], hour_col: usize, day_col: usize) {
    let mut row = format!("{:>w$} ", "all", w = hour_col - 1);
    for cells in all_day.iter().take(7) {
        let cell = match cells.first() {
            None => pad("", day_col),
            Some(ge) => {
                let body = format!("█ {}", ge.event.summary);
                let extra = if cells.len() > 1 {
                    format!(" +{}", cells.len() - 1)
                } else {
                    String::new()
                };
                let inner_w = day_col.saturating_sub(extra.len()).saturating_sub(1);
                let truncated = truncate_chars(&body, inner_w);
                let visible = format!("{}{}", truncated, extra);
                colorize_pad(&visible, ge.color, day_col)
            }
        };
        row.push_str(&cell);
    }
    println!("{}", row);
}

fn print_hour_row(hour: u32, timed: &[Vec<TimedSlot>], hour_col: usize, day_col: usize) {
    let cells: Vec<Cell> = (0..7)
        .map(|i| {
            let starting: Vec<&TimedSlot> = timed[i]
                .iter()
                .filter(|s| s.start.hour() == hour)
                .collect();
            match starting.first() {
                None => Cell::empty(),
                Some(s) => Cell::for_slot(s, starting.len() - 1, day_col),
            }
        })
        .collect();

    let any_wrap = cells.iter().any(|c| c.line2.is_some());

    let mut row1 = format!("{:>w$} ", format!("{:02}", hour), w = hour_col - 1);
    for c in &cells {
        row1.push_str(&c.render(&c.line1, day_col));
    }
    println!("{}", row1);

    if any_wrap {
        let mut row2 = " ".repeat(hour_col);
        for c in &cells {
            let text = c.line2.as_deref().unwrap_or("");
            row2.push_str(&c.render(text, day_col));
        }
        println!("{}", row2);
    }
}

struct Cell {
    line1: String,
    line2: Option<String>,
    color: Option<(u8, u8, u8)>,
}

impl Cell {
    fn empty() -> Self {
        Self {
            line1: String::new(),
            line2: None,
            color: None,
        }
    }

    fn for_slot(s: &TimedSlot, extras: usize, day_col: usize) -> Self {
        let prefix = minute_prefix(s.start);
        let suffix = duration_suffix(s.start, s.end);
        let extra = if extras > 0 {
            format!(" +{}", extras)
        } else {
            String::new()
        };

        // Single-line layout: "█ {prefix}{title}{suffix}{extra}", with 1 char trailing
        // space reserved between cells.
        let chrome_inline =
            2 + prefix.chars().count() + suffix.chars().count() + extra.chars().count();
        let title_w_inline = day_col.saturating_sub(chrome_inline).saturating_sub(1);

        let title = &s.ge.event.summary;
        let title_chars = title.chars().count();

        if title_chars <= title_w_inline {
            return Self {
                line1: format!("█ {}{}{}{}", prefix, title, suffix, extra),
                line2: None,
                color: Some(s.ge.color),
            };
        }

        // Wrap: line 1 carries `█ {prefix}{part1}`, line 2 carries `  {part2}{suffix}{extra}`.
        let line1_title_w = day_col.saturating_sub(2 + prefix.chars().count()).saturating_sub(1);
        let line2_title_w = day_col
            .saturating_sub(2 + suffix.chars().count() + extra.chars().count())
            .saturating_sub(1);

        if line1_title_w == 0 || line2_title_w == 0 {
            // Cell too narrow to wrap usefully; truncate.
            let truncated = truncate_chars(title, title_w_inline);
            return Self {
                line1: format!("█ {}{}{}{}", prefix, truncated, suffix, extra),
                line2: None,
                color: Some(s.ge.color),
            };
        }

        let (part1, part2) = split_title(title, line1_title_w);
        let part2_truncated = truncate_chars(&part2, line2_title_w);
        Self {
            line1: format!("█ {}{}", prefix, part1),
            line2: Some(format!("  {}{}{}", part2_truncated, suffix, extra)),
            color: Some(s.ge.color),
        }
    }

    fn render(&self, text: &str, day_col: usize) -> String {
        match self.color {
            Some(c) => colorize_pad(text, c, day_col),
            None => pad(text, day_col),
        }
    }
}

/// Split `title` so the first part fits in `line1_max` chars. Prefers a
/// space boundary; falls back to a hard break if no space exists.
fn split_title(title: &str, line1_max: usize) -> (String, String) {
    let chars: Vec<char> = title.chars().collect();
    if chars.len() <= line1_max {
        return (title.to_string(), String::new());
    }

    let break_at = (1..=line1_max).rev().find(|&i| chars[i - 1] == ' ');
    match break_at {
        Some(i) => {
            let p1: String = chars[..i].iter().collect::<String>().trim_end().to_string();
            let p2: String = chars[i..].iter().collect::<String>().trim_start().to_string();
            (p1, p2)
        }
        None => {
            let p1: String = chars[..line1_max].iter().collect();
            let p2: String = chars[line1_max..].iter().collect();
            (p1, p2)
        }
    }
}

struct TimedSlot<'a> {
    ge: &'a GridEvent<'a>,
    start: NaiveDateTime,
    end: NaiveDateTime,
}

fn weekday_short(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn monday_of(d: NaiveDate) -> NaiveDate {
    let dow = d.weekday().num_days_from_monday() as i64;
    d - Duration::days(dow)
}

fn local_dt(t: &EventTime) -> Option<NaiveDateTime> {
    match t {
        EventTime::Date(_) => None,
        EventTime::DateTimeUtc(dt) => Some(dt.with_timezone(&Local).naive_local()),
        EventTime::DateTimeFloating(dt) => Some(*dt),
        EventTime::DateTimeZoned { datetime, tzid } => Some(zoned_to_local(datetime, tzid)),
    }
}

fn local_date(t: &EventTime) -> NaiveDate {
    match t {
        EventTime::Date(d) => *d,
        EventTime::DateTimeUtc(dt) => dt.with_timezone(&Local).date_naive(),
        EventTime::DateTimeFloating(dt) => dt.date(),
        EventTime::DateTimeZoned { datetime, tzid } => zoned_to_local(datetime, tzid).date(),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        s.chars()
            .take(max.saturating_sub(1))
            .chain(std::iter::once('…'))
            .collect()
    }
}

fn pad(s: &str, w: usize) -> String {
    let truncated = truncate_chars(s, w);
    let len = truncated.chars().count();
    let mut out = truncated;
    for _ in len..w {
        out.push(' ');
    }
    out
}

fn colorize_pad(visible: &str, color: (u8, u8, u8), w: usize) -> String {
    let padded = pad(visible, w);
    let (r, g, b) = color;
    padded.truecolor(r, g, b).to_string()
}

/// Tiny ":30 " prefix for events that start at non-zero minutes; empty otherwise.
fn minute_prefix(start: NaiveDateTime) -> String {
    if start.minute() == 0 {
        String::new()
    } else {
        format!(":{:02} ", start.minute())
    }
}

/// Compact duration suffix: " 2h" for multi-hour blocks, " 30m" for sub-hour
/// non-aligned events, " +" for events that cross midnight, empty for the
/// common 1-hour-on-the-hour case.
fn duration_suffix(start: NaiveDateTime, end: NaiveDateTime) -> String {
    if start.date() != end.date() {
        return " +".to_string();
    }
    let mins = (end - start).num_minutes().max(0);
    if mins <= 60 {
        return String::new();
    }
    let hours = mins / 60;
    let leftover = mins % 60;
    if leftover == 0 {
        format!(" {}h", hours)
    } else {
        format!(" {}h{}", hours, leftover)
    }
}
