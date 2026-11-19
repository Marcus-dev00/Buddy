//! Persistent Scheduled Reminder Engine & Local Time Helpers:
//! - Background poller checks SQLite `reminders` every 2s and emits `buddy://reminder-due`.
//! - Zero external datetime crates: pure civil calendar math + Win32 local timezone bias.

use crate::db::{now_ms, MemoryDb};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize)]
pub struct ReminderDuePayload {
    pub id: i64,
    pub title: String,
    pub repeat_rule: Option<String>,
    pub fired_at_ms: i64,
}

/// Returns local UTC offset in milliseconds (e.g. +8h = +28_800_000 for Asia/Shanghai).
pub fn local_utc_offset_ms() -> i64 {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
        unsafe {
            let local = GetLocalTime();
            let utc = GetSystemTime();
            let to_min = |y: u16, mo: u16, d: u16, h: u16, mi: u16| -> i64 {
                let days = days_from_civil(y as i64, mo as u32, d as u32);
                days * 1440 + (h as i64) * 60 + (mi as i64)
            };
            let diff_min = to_min(local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
                - to_min(utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute);
            diff_min * 60_000
        }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Howard Hinnant's civil-from-days algorithm: converts days since 1970-01-01 to `(year, month, day)`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

/// Converts `(year, month, day)` to days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = month as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + (day as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + (doe as i64) - 719_468
}

/// Formats a UTC epoch millisecond timestamp as local `YYYY-MM-DD HH:MM (Weekday)`.
pub fn format_local_datetime(utc_ms: i64, tz_offset_ms: i64) -> String {
    const DAY_MS: i64 = 86_400_000;
    let local_ms = utc_ms + tz_offset_ms;
    let days = local_ms.div_euclid(DAY_MS);
    let rem_ms = local_ms.rem_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(days);
    let total_secs = rem_ms / 1000;
    let hh = total_secs / 3600;
    let mm = (total_secs % 3600) / 60;
    let weekday = match (days + 4).rem_euclid(7) {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    };
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} ({weekday})")
}

pub fn format_local_now() -> String {
    format_local_datetime(now_ms(), local_utc_offset_ms())
}

/// Parses either a relative `delay_minutes` or a local time string (`HH:MM` or `YYYY-MM-DD HH:MM`)
/// into a UTC epoch timestamp in milliseconds.
pub fn parse_fire_at_ms(
    delay_minutes: Option<f64>,
    at_local: Option<&str>,
    reference_utc_ms: i64,
    tz_offset_ms: i64,
) -> Result<i64, String> {
    if let Some(mins) = delay_minutes {
        if !mins.is_finite() || mins <= 0.0 || mins > 525_600.0 {
            return Err("delay_minutes must be between 0.1 and 525600.".to_string());
        }
        return Ok(reference_utc_ms + (mins * 60_000.0).round() as i64);
    }

    let raw = at_local
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Provide either `delay_minutes` or `at_time` (HH:MM or YYYY-MM-DD HH:MM).".to_string())?;

    const DAY_MS: i64 = 86_400_000;
    let normalized = raw.replace('T', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();

    if parts.len() == 1 {
        // Expect HH:MM
        let (hh, mm) = parse_hh_mm(parts[0])?;
        let local_now = reference_utc_ms + tz_offset_ms;
        let today_days = local_now.div_euclid(DAY_MS);
        let target_local = today_days * DAY_MS + (hh as i64) * 3_600_000 + (mm as i64) * 60_000;
        let mut target_utc = target_local - tz_offset_ms;
        if target_utc <= reference_utc_ms {
            target_utc += DAY_MS;
        }
        return Ok(target_utc);
    }

    if parts.len() >= 2 {
        let date_parts: Vec<&str> = parts[0].split('-').collect();
        if date_parts.len() != 3 {
            return Err(format!("Invalid date `{}` (expected YYYY-MM-DD HH:MM)", parts[0]));
        }
        let year: i64 = date_parts[0]
            .parse()
            .map_err(|_| format!("Invalid year in `{raw}`"))?;
        let month: u32 = date_parts[1]
            .parse()
            .map_err(|_| format!("Invalid month in `{raw}`"))?;
        let day: u32 = date_parts[2]
            .parse()
            .map_err(|_| format!("Invalid day in `{raw}`"))?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err(format!("Out-of-range calendar date in `{raw}`"));
        }
        let (hh, mm) = parse_hh_mm(parts[1])?;
        let days = days_from_civil(year, month, day);
        let target_local = days * DAY_MS + (hh as i64) * 3_600_000 + (mm as i64) * 60_000;
        return Ok(target_local - tz_offset_ms);
    }

    Err(format!("Unsupported time format `{raw}`"))
}

fn parse_hh_mm(s: &str) -> Result<(u32, u32), String> {
    let segs: Vec<&str> = s.split(':').collect();
    if segs.len() < 2 {
        return Err(format!("Invalid clock time `{s}` (expected HH:MM)"));
    }
    let hh: u32 = segs[0]
        .parse()
        .map_err(|_| format!("Invalid hour in `{s}`"))?;
    let mm: u32 = segs[1]
        .parse()
        .map_err(|_| format!("Invalid minute in `{s}`"))?;
    if hh > 23 || mm > 59 {
        return Err(format!("Clock time out of range `{s}`"));
    }
    Ok((hh, mm))
}

pub fn start_reminder_loop(app: AppHandle, db: Arc<MemoryDb>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = now_ms();
            let tz = local_utc_offset_ms();
            if let Ok(due_items) = db.pop_due_reminders(now, tz) {
                for item in due_items {
                    let _ = app.emit(
                        "buddy://reminder-due",
                        ReminderDuePayload {
                            id: item.id,
                            title: item.title,
                            repeat_rule: item.repeat_rule,
                            fired_at_ms: now,
                        },
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relative_and_local_clock_reminders_accurately() {
        // 2026-11-19 10:00 UTC => 2026-11-19 18:00 UTC+8
        let tz = 8 * 3_600_000i64;
        let base_days = days_from_civil(2026, 11, 19);
        let ref_utc = base_days * 86_400_000 + 10 * 3_600_000;

        assert_eq!(
            format_local_datetime(ref_utc, tz),
            "2026-11-19 18:00 (Thu)"
        );

        // Relative +25 min
        let rel = parse_fire_at_ms(Some(25.0), None, ref_utc, tz).unwrap();
        assert_eq!(rel, ref_utc + 25 * 60_000);

        // Future HH:MM today -> 19:30 today
        let same_day = parse_fire_at_ms(None, Some("19:30"), ref_utc, tz).unwrap();
        assert_eq!(
            format_local_datetime(same_day, tz),
            "2026-11-19 19:30 (Thu)"
        );

        // Past HH:MM today -> rolls over to tomorrow 09:00
        let next_day = parse_fire_at_ms(None, Some("09:00"), ref_utc, tz).unwrap();
        assert_eq!(
            format_local_datetime(next_day, tz),
            "2026-11-20 09:00 (Fri)"
        );

        // Explicit full date
        let full = parse_fire_at_ms(None, Some("2026-12-01 08:15"), ref_utc, tz).unwrap();
        assert_eq!(
            format_local_datetime(full, tz),
            "2026-12-01 08:15 (Tue)"
        );
    }
}
