use anyhow::{Context, Result};
use caldir_core::calendar::Calendar;
use caldir_core::event::{Event, EventTime};
use chrono::{Duration, Utc};
use dialoguer::{Input, Select};
use owo_colors::OwoColorize;

use crate::commands::new::{apply_duration, parse_datetime};
use crate::utils::date::format_datetime;

#[derive(Clone)]
struct Match {
    calendar: Calendar,
    event: Event,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    query: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    start: Option<String>,
    end: Option<String>,
    duration: Option<String>,
    location: Option<String>,
    calendars: Vec<Calendar>,
) -> Result<()> {
    if summary.is_none()
        && description.is_none()
        && start.is_none()
        && end.is_none()
        && duration.is_none()
        && location.is_none()
    {
        anyhow::bail!(
            "No edits specified. Use one or more of:\n  \
             --summary, --description, --start, --end, --duration, --location"
        );
    }
    if end.is_some() && duration.is_some() {
        anyhow::bail!("Use either --end or --duration, not both.");
    }

    let query = match query {
        Some(q) => q,
        None => Input::<String>::new()
            .with_prompt("  Search")
            .interact_text()?,
    };

    let q_lower = query.to_lowercase();
    let mut matches: Vec<Match> = Vec::new();
    for cal in &calendars {
        for ce in cal.events()? {
            if ce.event.summary.to_lowercase().contains(&q_lower) {
                matches.push(Match {
                    calendar: cal.clone(),
                    event: ce.event,
                });
            }
        }
    }

    if matches.is_empty() {
        anyhow::bail!("No events match \"{}\"", query);
    }

    let chosen = if matches.len() == 1 {
        matches.into_iter().next().unwrap()
    } else {
        let items: Vec<String> = matches.iter().map(format_pick_item).collect();
        let idx = Select::new()
            .with_prompt("  Which event?")
            .items(&items)
            .default(0)
            .interact()?;
        matches.into_iter().nth(idx).unwrap()
    };

    let (target, save_uid, save_rid) = resolve_target(&chosen)?;

    let updated = apply_edits(
        &target,
        summary,
        description,
        start.as_deref(),
        end.as_deref(),
        duration.as_deref(),
        location,
    )?;

    chosen
        .calendar
        .update_event(&save_uid, save_rid.as_ref(), &updated)?;

    println!();
    if updated.summary != target.summary {
        println!(
            "  {} {} → {}",
            "✓".green(),
            target.summary,
            updated.summary
        );
    } else {
        println!("  {} {}", "✓".green(), updated.summary);
    }
    println!("{}", "Remember to run: caldir push".dimmed());

    Ok(())
}

fn format_pick_item(m: &Match) -> String {
    let kind = if m.event.recurrence.is_some() {
        " (series)"
    } else if m.event.recurrence_id.is_some() {
        " (override)"
    } else {
        ""
    };
    format!(
        "{}  {}  [{}]{}",
        format_datetime(&m.event.start),
        m.event.summary,
        m.calendar.slug,
        kind
    )
}

/// Resolve which event to actually save, prompting for recurring scope when needed.
/// Returns the event to mutate plus the (uid, recurrence_id) used to address it on save.
fn resolve_target(chosen: &Match) -> Result<(Event, String, Option<EventTime>)> {
    if chosen.event.recurrence.is_none() {
        // Single event or existing override — edit directly.
        return Ok((
            chosen.event.clone(),
            chosen.event.uid.clone(),
            chosen.event.recurrence_id.clone(),
        ));
    }

    // Master event: ask whether to edit the series or a single occurrence.
    let items = ["The whole series", "A specific occurrence"];
    let scope_idx = Select::new()
        .with_prompt("  This is a recurring event. Edit:")
        .items(&items)
        .default(0)
        .interact()?;

    if scope_idx == 0 {
        return Ok((
            chosen.event.clone(),
            chosen.event.uid.clone(),
            chosen.event.recurrence_id.clone(),
        ));
    }

    let date_input: String = Input::new()
        .with_prompt("  Which occurrence? (e.g. \"tomorrow\", \"may 10\")")
        .interact_text()?;
    let target_time = parse_datetime(&date_input)?;
    let occurrence = find_occurrence(&chosen.calendar, &chosen.event, &target_time)?;
    Ok((
        occurrence.clone(),
        occurrence.uid,
        occurrence.recurrence_id,
    ))
}

/// Expand the master series in a window around `target_time` and return the closest occurrence.
fn find_occurrence(cal: &Calendar, master: &Event, target_time: &EventTime) -> Result<Event> {
    let target_utc = target_time
        .to_utc()
        .context("Could not resolve target date to a UTC instant")?;
    let from = target_utc - Duration::hours(12);
    let to = target_utc + Duration::hours(36);

    cal.events_in_range(from, to)?
        .into_iter()
        .filter(|e| e.uid == master.uid && e.recurrence_id.is_some())
        .min_by_key(|e| {
            e.start
                .to_utc()
                .map(|s| (s - target_utc).num_seconds().abs())
                .unwrap_or(i64::MAX)
        })
        .context("No matching occurrence found near that date")
}

fn apply_edits(
    base: &Event,
    summary: Option<String>,
    description: Option<String>,
    start: Option<&str>,
    end: Option<&str>,
    duration: Option<&str>,
    location: Option<String>,
) -> Result<Event> {
    let mut updated = base.clone();

    if let Some(s) = summary {
        updated.summary = s;
    }
    if let Some(d) = description {
        updated.description = if d.is_empty() { None } else { Some(d) };
    }
    if let Some(l) = location {
        updated.location = if l.is_empty() { None } else { Some(l) };
    }

    if let Some(s_str) = start {
        let new_start = parse_datetime(s_str)?;
        let original_dur = duration_between(&base.start, &base.end);
        updated.start = new_start.clone();
        if end.is_none() && duration.is_none() {
            updated.end = match original_dur {
                Some(d) => shift_event_time(&new_start, d),
                None => new_start,
            };
        }
    }

    if let Some(e_str) = end {
        updated.end = parse_datetime(e_str)?;
    } else if let Some(d_str) = duration {
        updated.end = apply_duration(&updated.start, d_str)?;
    }

    updated.updated = Some(Utc::now());
    updated.sequence = Some(base.sequence.unwrap_or(0) + 1);

    Ok(updated)
}

fn shift_event_time(et: &EventTime, by: Duration) -> EventTime {
    match et {
        EventTime::Date(d) => EventTime::Date(*d + by),
        EventTime::DateTimeFloating(dt) => EventTime::DateTimeFloating(*dt + by),
        EventTime::DateTimeUtc(dt) => EventTime::DateTimeUtc(*dt + by),
        EventTime::DateTimeZoned { datetime, tzid } => EventTime::DateTimeZoned {
            datetime: *datetime + by,
            tzid: tzid.clone(),
        },
    }
}

fn duration_between(start: &EventTime, end: &EventTime) -> Option<Duration> {
    match (start, end) {
        (EventTime::Date(a), EventTime::Date(b)) => Some(*b - *a),
        (EventTime::DateTimeFloating(a), EventTime::DateTimeFloating(b)) => Some(*b - *a),
        (EventTime::DateTimeUtc(a), EventTime::DateTimeUtc(b)) => Some(*b - *a),
        (
            EventTime::DateTimeZoned { datetime: a, .. },
            EventTime::DateTimeZoned { datetime: b, .. },
        ) => Some(*b - *a),
        _ => match (start.to_utc(), end.to_utc()) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        },
    }
}
