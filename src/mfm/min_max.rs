//! 前日の最高・最低気温のノートの書式。ピン留めしたノートを差し替える形で使う。
//!
//! ```text
//! 昨日(10/21)の気温(多治見)     1 行目: ヘッダ。日付と地点名
//! 最高: 26.8 ℃ (14:30)
//! 最低: 19.4 ℃ (05:40)
//! ```

use chrono::{NaiveDate, NaiveTime};
use regex::Regex;
use std::sync::LazyLock;

use super::apply_replacements;
use crate::config::Replace;
use crate::jma::amedas::Extreme;

/// ヘッダの、地点名より前のところ。日付まで含むので、どの日のノートかもこれで見分ける。
fn header_prefix(date: NaiveDate) -> String {
    format!("昨日({})の気温(", date.format("%m/%d"))
}

fn header(date: NaiveDate, station: &str) -> String {
    format!("{}{station})", header_prefix(date))
}

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^昨日\(\d{2}/\d{2}\)の気温\(").expect("a valid regex"));

/// bot の前日気温のノートか。
pub fn is_mine(text: &str) -> bool {
    HEADER.is_match(text)
}

/// `date` の気温のノートか。
pub fn is_for(text: &str, date: NaiveDate) -> bool {
    text.starts_with(&header_prefix(date))
}

/// 本文を組み立てる。気温は気象庁のアメダスのページに合わせて小数 1 桁にする（15.0 を 15 と書かない）。
pub fn render(
    date: NaiveDate,
    station: &str,
    max: &Extreme,
    min: &Extreme,
    replacements: &[Replace],
) -> String {
    let line = |label: &str, extreme: &Extreme| {
        format!(
            "{label}: {:.1} ℃ ({})",
            extreme.temperature,
            clock(extreme.time)
        )
    };
    let body = format!("{}\n{}", line("最高", max), line("最低", min));
    format!(
        "{}\n{}",
        header(date, station),
        apply_replacements(&body, replacements)
    )
}

/// 起時の表記。気象庁は日界の観測値をその日にだけ数える（気象観測統計指針 2.2.2）ので、起時の 00:00 は日界の 24:00 にあたる。
/// 気象庁と同じく 24:00 と書く。00:00 と書くと、その日の始まりと読める。
fn clock(time: NaiveTime) -> String {
    if time == NaiveTime::MIN {
        "24:00".to_owned()
    } else {
        time.format("%H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str, temperature: f64) -> Extreme {
        Extreme {
            temperature,
            time: value.parse().unwrap(),
        }
    }

    fn date(value: &str) -> NaiveDate {
        value.parse().unwrap()
    }

    #[test]
    fn renders_the_header_and_both_extremes() {
        assert_eq!(
            render(
                date("2026-10-21"),
                "多治見",
                &at("14:30:00", 26.8),
                &at("05:40:00", 19.4),
                &[]
            ),
            "昨日(10/21)の気温(多治見)\n最高: 26.8 ℃ (14:30)\n最低: 19.4 ℃ (05:40)"
        );
    }

    #[test]
    fn whole_degrees_keep_one_decimal() {
        assert_eq!(
            render(
                date("2026-10-21"),
                "多治見",
                &at("14:30:00", 15.0),
                &at("05:40:00", -4.0),
                &[]
            ),
            "昨日(10/21)の気温(多治見)\n最高: 15.0 ℃ (14:30)\n最低: -4.0 ℃ (05:40)"
        );
    }

    #[test]
    fn an_extreme_at_the_day_boundary_is_written_as_24_00() {
        // 留萌の 2026-10-23。最低気温の起時は、気象庁の確定値でも 24 時 00 分。
        assert_eq!(
            render(
                date("2026-10-23"),
                "留萌",
                &at("11:56:00", 14.1),
                &at("00:00:00", 6.9),
                &[]
            ),
            "昨日(10/23)の気温(留萌)\n最高: 14.1 ℃ (11:56)\n最低: 6.9 ℃ (24:00)"
        );
    }

    #[test]
    fn replacements_do_not_touch_the_header() {
        let replacements = [Replace {
            pattern: Regex::new(r"多治見|33\.4 ℃").unwrap(),
            replacement: "X".to_owned(),
        }];
        assert_eq!(
            render(
                date("2026-10-21"),
                "多治見",
                &at("14:30:00", 33.4),
                &at("05:40:00", 19.4),
                &replacements
            ),
            "昨日(10/21)の気温(多治見)\n最高: X (14:30)\n最低: 19.4 ℃ (05:40)"
        );
    }

    #[test]
    fn the_header_identifies_the_note_and_its_date() {
        let text = render(
            date("2026-10-21"),
            "多治見",
            &at("14:30:00", 26.8),
            &at("05:40:00", 19.4),
            &[],
        );
        assert!(is_mine(&text));
        assert!(is_for(&text, date("2026-10-21")));
        assert!(!is_for(&text, date("2026-10-22")));
        assert!(!is_mine("10/22 天気予報(美濃地方)"));
    }
}
