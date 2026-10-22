//! 観測値のノートの書式と、その読み戻し。
//!
//! ```text
//! 🌡 14.3 ℃ 💨 0.8 m/s                                   1 行目: 気温と風速
//! ☀ 0 min/h 🌧 0.0 mm/h                                 2 行目: 日照と降水量
//! $[bg.color=ff2800 $[fg.color=ffffff 大雨・暴風警報]]   3 行目（任意）: 発表中の警報・注意報をレベルごとに
//! (多治見 21:00)                                        最終行: フッタ。地点名と観測時刻
//! ```

use chrono::{DateTime, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use regex::Regex;
use std::sync::LazyLock;

use super::apply_replacements;
use crate::config::Replace;
use crate::jma::amedas::Reading;
use crate::jma::warning::Level;
use crate::jma::{Kind, Observation};
use crate::time::JST;

/// 本文を組み立てる。`warnings` は発表中の種別（表の順）で、空なら行を出さない。
/// 桁数は気象庁のアメダスのページ（小数 1 桁）に合わせる。日照だけは時間を分に直すので整数にする。
pub fn render(
    station: &str,
    observation: &Observation,
    warnings: &[Kind],
    replacements: &[Replace],
) -> String {
    let mut body = format!(
        "{} {}\n{} {}",
        value("🌡", observation.temperature, 1, "℃"),
        value("💨", observation.wind_speed, 1, "m/s"),
        value("☀", observation.sunshine_minutes, 0, "min/h"),
        value("🌧", observation.precipitation_1h, 1, "mm/h"),
    );
    if let Some(line) = warning_line(warnings) {
        body.push('\n');
        body.push_str(&line);
    }
    format!(
        "{}\n{}",
        apply_replacements(&body, replacements),
        footer(station, observation.time.time())
    )
}

/// `🌡 14.3 ℃`。値は小数 `decimals` 桁に固定する（7.0 を 7 と書かない）。値が無いときは、気象庁のアメダスのページに合わせて
/// 休止中と欠測を分ける。項目が無ければ、原因を断定しない `---` にする。
fn value(icon: &str, reading: Reading, decimals: usize, unit: &str) -> String {
    match reading {
        Reading::Value(value) => format!("{icon} {value:.decimals$} {unit}"),
        Reading::Suspended => format!("{icon} 休止中"),
        Reading::Missing => format!("{icon} 欠測"),
        Reading::Absent => format!("{icon} ---"),
    }
}

/// 発表中の種別をレベルの高い順にまとめた 1 行。`大雨・暴風警報` のように種別を・で連ね、レベルの色で塗る。
pub fn warning_line(kinds: &[Kind]) -> Option<String> {
    let badges: Vec<String> = [
        Level::Emergency,
        Level::Urgent,
        Level::Warning,
        Level::Advisory,
    ]
    .into_iter()
    .filter_map(|level| {
        let names: Vec<&str> = kinds
            .iter()
            .filter(|kind| kind.level == level)
            .map(|kind| kind.name)
            .collect();
        if names.is_empty() {
            return None;
        }
        let (background, foreground) = colors(level);
        Some(badge(
            background,
            foreground,
            &format!("{}{}", names.join("・"), level.suffix()),
        ))
    })
    .collect();
    (!badges.is_empty()).then(|| badges.join(" "))
}

/// レベルの色（背景, 文字）。気象庁のサイトの配色に合わせている。
fn colors(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::Advisory => ("f2e700", "000000"),
        Level::Warning => ("ff2800", "ffffff"),
        Level::Urgent => ("aa00aa", "ffffff"),
        Level::Emergency => ("0c000c", "ffffff"),
    }
}

/// 色付きの帯。
fn badge(background: &str, foreground: &str, text: &str) -> String {
    format!("$[bg.color={background} $[fg.color={foreground} {text}]]")
}

fn footer(station: &str, time: NaiveTime) -> String {
    format!("({station} {})", time.format("%H:%M"))
}

static FOOTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n\((.+) (\d{2}:\d{2})\)$").expect("a valid regex"));

/// `station` の観測ノートなら、その観測時刻（10 分枠の始まり、JST）。地点名まで見るのは、bot のアカウントで手で投稿した
/// `(会議 23:00)` のような行で終わるノートを観測ノートと読み違え、投稿済みの枠をもう一度投稿しないため。
///
/// フッタには時刻しか無いので、日付はノートの作成日時から取る。日付が変わった直後に前日の枠を投稿すると
/// 作成日時の日付とずれるので、枠が作成日時より後になるなら前日とみなす。
pub fn posted_slot(text: &str, station: &str, created_at: DateTime<Utc>) -> Option<NaiveDateTime> {
    let captures = FOOTER.captures(text)?;
    if captures[1] != *station {
        return None;
    }
    let time = NaiveTime::parse_from_str(&captures[2], "%H:%M").ok()?;
    let created_at = created_at.with_timezone(&JST).naive_local();
    let slot = created_at.date().and_time(time);
    Some(if slot > created_at {
        slot - TimeDelta::days(1)
    } else {
        slot
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        Observation {
            time: "2026-10-22T21:00:00".parse().unwrap(),
            temperature: Reading::Value(14.3),
            precipitation_1h: Reading::Value(0.0),
            sunshine_minutes: Reading::Value(30.0),
            wind_speed: Reading::Value(0.8),
            max_temperature: None,
            min_temperature: None,
        }
    }

    fn kinds(codes: &[&str]) -> Vec<Kind> {
        codes
            .iter()
            .map(|code| Kind::from_code(code).unwrap())
            .collect()
    }

    #[test]
    fn renders_values_and_the_footer() {
        assert_eq!(
            render("多治見", &observation(), &[], &[]),
            "🌡 14.3 ℃ 💨 0.8 m/s\n☀ 30 min/h 🌧 0.0 mm/h\n(多治見 21:00)"
        );
    }

    #[test]
    fn whole_values_keep_their_decimals() {
        // 帯広の 2026-10-25 00:00 の気温 7.0 を、7 と書いていた。
        let observation = Observation {
            temperature: Reading::Value(7.0),
            wind_speed: Reading::Value(2.0),
            ..observation()
        };
        assert_eq!(
            render("多治見", &observation, &[], &[]),
            "🌡 7.0 ℃ 💨 2.0 m/s\n☀ 30 min/h 🌧 0.0 mm/h\n(多治見 21:00)"
        );
    }

    #[test]
    fn values_without_a_number_follow_the_quality_flag() {
        let observation = Observation {
            temperature: Reading::Suspended,
            wind_speed: Reading::Absent,
            sunshine_minutes: Reading::Missing,
            ..observation()
        };
        assert_eq!(
            render("多治見", &observation, &[], &[]),
            "🌡 休止中 💨 ---\n☀ 欠測 🌧 0.0 mm/h\n(多治見 21:00)"
        );
    }

    #[test]
    fn warnings_are_grouped_by_level_from_the_highest() {
        let line = warning_line(&kinds(&["43", "10", "29", "08", "05", "07", "14"])).unwrap();
        assert_eq!(
            line,
            "$[bg.color=aa00aa $[fg.color=ffffff 大雨危険警報]] \
             $[bg.color=ff2800 $[fg.color=ffffff 高潮・暴風・波浪警報]] \
             $[bg.color=f2e700 $[fg.color=000000 大雨・土砂災害・雷注意報]]"
        );
        assert_eq!(
            warning_line(&kinds(&["33"])).unwrap(),
            "$[bg.color=0c000c $[fg.color=ffffff 大雨特別警報]]"
        );
        assert!(warning_line(&[]).is_none());
    }

    #[test]
    fn the_warning_line_is_between_the_values_and_the_footer() {
        let text = render("多治見", &observation(), &kinds(&["15"]), &[]);
        assert_eq!(
            text,
            "🌡 14.3 ℃ 💨 0.8 m/s\n☀ 30 min/h 🌧 0.0 mm/h\n$[bg.color=f2e700 $[fg.color=000000 強風注意報]]\n(多治見 21:00)"
        );
    }

    #[test]
    fn replacements_do_not_touch_the_footer() {
        let replacements = [Replace {
            pattern: Regex::new(r"\d+:\d+|14\.3 ℃").unwrap(),
            replacement: "X".to_owned(),
        }];
        assert_eq!(
            render("多治見", &observation(), &[], &replacements),
            "🌡 X 💨 0.8 m/s\n☀ 30 min/h 🌧 0.0 mm/h\n(多治見 21:00)"
        );
    }

    #[test]
    fn the_slot_is_read_back_from_the_footer_and_the_creation_time() {
        let created = |value: &str| {
            DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&Utc)
        };
        let text = render("多治見", &observation(), &[], &[]);
        assert_eq!(
            posted_slot(&text, "多治見", created("2026-10-22T21:02:10+09:00")),
            Some(observation().time)
        );
        // 日付が変わった直後に投稿した前日の枠。
        let late = Observation {
            time: "2026-10-22T23:50:00".parse().unwrap(),
            ..observation()
        };
        assert_eq!(
            posted_slot(
                &render("多治見", &late, &[], &[]),
                "多治見",
                created("2026-10-23T00:01:30+09:00")
            ),
            Some(late.time)
        );
        assert!(
            posted_slot(
                "10/22 天気予報(美濃地方)\n晴れ",
                "多治見",
                created("2026-10-22T21:02:10+09:00")
            )
            .is_none()
        );
        assert!(posted_slot("", "多治見", created("2026-10-22T21:02:10+09:00")).is_none());
        // 別の地点のフッタや、手で投稿した同じ形の行は、観測ノートとみなさない。
        assert!(posted_slot(&text, "岐阜", created("2026-10-22T21:02:10+09:00")).is_none());
        assert!(
            posted_slot(
                "お知らせ\n(会議 23:00)",
                "多治見",
                created("2026-10-22T21:02:10+09:00")
            )
            .is_none()
        );
    }
}
