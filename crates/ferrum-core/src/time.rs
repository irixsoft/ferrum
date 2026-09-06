/// SQLite's `datetime('now')` is UTC but carries no zone, and a browser reads that as local time.
pub fn utc(stamp: String) -> String {
    match stamp.split_once(' ') {
        Some((date, time)) => format!("{date}T{time}Z"),
        None => stamp,
    }
}

pub fn utc_opt(stamp: Option<String>) -> Option<String> {
    stamp.map(utc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sqlite_timestamp_becomes_unambiguous() {
        assert_eq!(utc("2026-09-01 03:07:24".into()), "2026-09-01T03:07:24Z");
    }

    #[test]
    fn an_already_marked_timestamp_is_left_alone() {
        for already in ["2026-09-01T03:07:24Z", "2026-09-01T03:07:24+00:00", ""] {
            assert_eq!(utc(already.into()), already);
        }
    }

    #[test]
    fn the_optional_form_passes_none_through() {
        assert_eq!(utc_opt(None), None);
        assert_eq!(
            utc_opt(Some("2026-09-01 03:07:24".into())).as_deref(),
            Some("2026-09-01T03:07:24Z")
        );
    }
}
