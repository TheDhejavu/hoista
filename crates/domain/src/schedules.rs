//! Checks cron schedules before they're saved or sent to a provider: five
//! fields, in UTC, in the form providers that run cron jobs accept.

/// The most schedules one resource may have.
const MAX_SCHEDULES: usize = 100;
/// The longest schedule accepted.
const MAX_SCHEDULE_BYTES: usize = 256;
/// The smallest and largest number each field takes: minute, hour, day of
/// the month, month and day of the week (1 is Sunday).
const FIELD_RANGES: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (1, 7)];
const DAY_OF_MONTH: usize = 2;
const MONTH: usize = 3;
const DAY_OF_WEEK: usize = 4;

/// Whether every schedule is valid and none repeats.
///
/// Each field takes `*`, a number, a range (`1-5`), a list (`1,15`) and a
/// step (`*/15`). Months and days can be names (`JAN`, `MON`). The day of
/// the month also takes `L` (last day), `LW` (last weekday) and `15W` (the
/// weekday nearest the 15th); the day of the week takes `5L` (the last
/// Thursday) and `2#1` (the first Monday).
pub fn valid(schedules: &[String]) -> bool {
    schedules.len() <= MAX_SCHEDULES
        && schedules
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == schedules.len()
        && schedules.iter().all(|value| {
            if value.len() > MAX_SCHEDULE_BYTES
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return false;
            }
            let fields: Vec<_> = value.split(' ').collect();
            fields.len() == 5
                && fields
                    .iter()
                    .zip(FIELD_RANGES)
                    .enumerate()
                    .all(|(index, (field, (min, max)))| field_valid(field, index, min, max))
        })
}

fn field_valid(field: &str, index: usize, min: u32, max: u32) -> bool {
    let upper = field.to_ascii_uppercase();
    let field = upper.as_str();
    if index == DAY_OF_MONTH {
        if matches!(field, "L" | "LW") {
            return true;
        }
        if let Some(day) = field.strip_suffix('W') {
            return number(day).is_some_and(|day| (1..=31).contains(&day));
        }
    }
    if index == DAY_OF_WEEK {
        if let Some(day) = field.strip_suffix('L') {
            return field_number(day, index).is_some_and(|day| (1..=7).contains(&day));
        }
        if let Some((day, nth)) = field.split_once('#') {
            return field_number(day, index).is_some_and(|day| (1..=7).contains(&day))
                && number(nth).is_some_and(|n| (1..=5).contains(&n));
        }
    }
    field
        .split(',')
        .all(|term| term_valid(term, index, min, max))
}

/// A number, or for months and days of the week, a name.
fn field_number(value: &str, index: usize) -> Option<u32> {
    let names: &[&str] = match index {
        MONTH => &[
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
        ],
        DAY_OF_WEEK => &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"],
        _ => &[],
    };
    names
        .iter()
        .position(|name| *name == value)
        .map(|n| n as u32 + 1)
        .or_else(|| number(value))
}

fn number(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn term_valid(term: &str, index: usize, min: u32, max: u32) -> bool {
    let mut step = term.split('/');
    let Some(base) = step.next() else {
        return false;
    };
    if let Some(value) = step.next()
        && !number(value).is_some_and(|n| n > 0 && n <= max - min + 1)
    {
        return false;
    }
    if step.next().is_some() {
        return false;
    }
    if base == "*" {
        return true;
    }
    let mut range = base.split('-');
    let Some(start) = range.next().and_then(|value| field_number(value, index)) else {
        return false;
    };
    if !(min..=max).contains(&start) {
        return false;
    }
    if let Some(end) = range.next()
        && !field_number(end, index).is_some_and(|n| n >= start && n <= max)
    {
        return false;
    }
    range.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::valid;

    #[test]
    fn cron_values_are_bounded_and_validated_before_writes() {
        assert!(valid(&[]));
        assert!(valid(&[
            "*/15 0-6,18 * 1,12 1-5".into(),
            "0 12 * * *".into()
        ]));
        for value in [
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 0",
            "*/0 * * * *",
            "2-1 * * * *",
            "* * * *",
            "*  * * * *",
            "* * * * *\n",
            "* * * * MONDAY",
            "1/2/3 * * * *",
            "1, * * * *",
        ] {
            assert!(!valid(&[value.into()]), "{value}");
        }
        assert!(!valid(&vec!["* * * * *".into(); 101]));
        assert!(!valid(&["* * * * *".into(), "* * * * *".into()]));
    }
}
