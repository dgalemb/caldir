use anyhow::Result;
use caldir_core::calendar::Calendar;
use chrono::{DateTime, Duration, Local, Utc};
use owo_colors::OwoColorize;

use crate::render::{format_event_line, render_participation_status};
use crate::render_grid::{GridEvent, color_for, render as render_grid};
use crate::utils::date::{format_date_only, start_of_today};

pub fn run(
    calendars: Vec<Calendar>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    grid_view: bool,
) -> Result<()> {
    let today = start_of_today();
    let from = from.unwrap_or(today);
    let to = to.unwrap_or(today + Duration::days(3));

    // (cal_slug, account_email, event, color)
    let mut all_events: Vec<(String, Option<String>, caldir_core::event::Event, (u8, u8, u8))> =
        Vec::new();

    for cal in &calendars {
        let email = cal.account_email().map(String::from);
        let color = color_for(&cal.slug, cal.config.color.as_deref());
        let events = cal.events_in_range(from, to)?;
        for event in events {
            all_events.push((cal.slug.clone(), email.clone(), event, color));
        }
    }

    all_events.sort_by(|a, b| a.2.start.to_utc().cmp(&b.2.start.to_utc()));

    if all_events.is_empty() {
        println!("{}", "No events found".dimmed());
        return Ok(());
    }

    if grid_view {
        let grid_events: Vec<GridEvent> = all_events
            .iter()
            .map(|(_, _, event, color)| GridEvent {
                event,
                color: *color,
            })
            .collect();
        let from_date = from.with_timezone(&Local).date_naive();
        let to_date = to.with_timezone(&Local).date_naive();
        render_grid(&grid_events, from_date, to_date);
        return Ok(());
    }

    let mut current_date: Option<String> = None;
    for (cal_slug, email, event, _color) in &all_events {
        let date_label = format_date_only(&event.start);
        if current_date.as_ref() != Some(&date_label) {
            if current_date.is_some() {
                println!();
            }
            println!("{}", date_label.bold());
            current_date = Some(date_label);
        }

        let invite_indicator = email
            .as_deref()
            .filter(|e| event.is_invite_for(e))
            .and_then(|e| event.my_status(e))
            .map(|status| format!(" ({})", render_participation_status(status)))
            .unwrap_or_default();
        println!("{}", format_event_line(event, cal_slug, &invite_indicator));
    }

    Ok(())
}
