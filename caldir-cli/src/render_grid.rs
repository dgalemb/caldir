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
    let day_col = ((term_width.saturating_sub(hour_col)) / 7).clamp(8, 22);

    let mut week = monday_of(from);
    let mut first = true;
    while week <= to {
        if !first {
            println!();
        }
        first = false;
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

    print_month_label(week_start, week_end);
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

fn print_month_label(start: NaiveDate, end: NaiveDate) {
    let label = if start.month() == end.month() {
        start.format("%B %Y").to_string()
    } else {
        format!("{} – {}", start.format("%b"), end.format("%b %Y"))
    };
    println!("{}", label.bold());
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
    let mut row = format!("{:>w$} ", format!("{:02}", hour), w = hour_col - 1);
    for slots in timed.iter().take(7) {
        let starting: Vec<&TimedSlot> = slots.iter().filter(|s| s.start.hour() == hour).collect();
        let cell = match starting.first() {
            None => pad("", day_col),
            Some(s) => {
                let dur = format_dur(s.start, s.end);
                let extra = if starting.len() > 1 {
                    format!(" +{}", starting.len() - 1)
                } else {
                    String::new()
                };
                let dur_part = if dur.is_empty() {
                    String::new()
                } else {
                    format!(" {}", dur)
                };
                let suffix_w = dur_part.len() + extra.len();
                let title_w = day_col
                    .saturating_sub(2)
                    .saturating_sub(suffix_w)
                    .saturating_sub(1);
                let title = truncate_chars(&s.ge.event.summary, title_w);
                let visible = format!("█ {}{}{}", title, dur_part, extra);
                colorize_pad(&visible, s.ge.color, day_col)
            }
        };
        row.push_str(&cell);
    }
    println!("{}", row);
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

fn format_dur(start: NaiveDateTime, end: NaiveDateTime) -> String {
    if start.date() != end.date() {
        return format!("{:02}:{:02}+", start.hour(), start.minute());
    }
    let dur_min = (end - start).num_minutes();
    if dur_min <= 60 && start.minute() == 0 && end.minute() == 0 {
        return String::new();
    }
    if start.minute() == 0 && end.minute() == 0 {
        format!("{:02}-{:02}", start.hour(), end.hour())
    } else {
        format!(
            "{:02}:{:02}-{:02}:{:02}",
            start.hour(),
            start.minute(),
            end.hour(),
            end.minute()
        )
    }
}
