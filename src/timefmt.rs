use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn parse_rfc3339(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

pub fn format_short(dt: OffsetDateTime) -> String {
    // Example: "Jan20 00:06"
    let month = match dt.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    format!("{month}{:02} {:02}:{:02}", dt.day(), dt.hour(), dt.minute())
}

pub fn format_age(dt: OffsetDateTime) -> String {
    let now = OffsetDateTime::now_utc();
    let delta = now - dt;
    let secs = delta.whole_seconds().max(0);

    if secs < 60 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 14 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 52 {
        return format!("{weeks}w");
    }
    let years = weeks / 52;
    format!("{years}y")
}
