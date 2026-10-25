//! 天気予報のノートの書式。ピン留めしたノートを差し替える形で使う。
//!
//! ```text
//! 10/23 天気予報(美濃地方)        1 行目: ヘッダ。日付と区域名
//! $[x2 ☀]→$[x2 ☁] 晴れ後曇り      天気の絵文字と名称
//! 降水確率: 0→0→0→10 %           その日の時間帯ごと
//! 気温: ↓14 ↑22 ℃                最低（当日の発表には無い）と最高
//! ```

use chrono::NaiveDate;
use regex::Regex;
use std::sync::LazyLock;

use super::apply_replacements;
use crate::config::Replace;
use crate::jma::DailyForecast;
use crate::jma::weather::{Pattern, Weather};

/// ヘッダの、区域名より前のところ。日付まで含むので、どの日のノートかもこれで見分ける。
fn header_prefix(date: NaiveDate) -> String {
    format!("{} 天気予報(", date.format("%m/%d"))
}

fn header(date: NaiveDate, area_name: &str) -> String {
    format!("{}{area_name})", header_prefix(date))
}

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{2}/\d{2} 天気予報\(").expect("a valid regex"));

/// bot の天気予報のノートか。
pub fn is_mine(text: &str) -> bool {
    HEADER.is_match(text)
}

/// `date` の天気予報のノートか。
pub fn is_for(text: &str, date: NaiveDate) -> bool {
    text.starts_with(&header_prefix(date))
}

/// 本文を組み立てる。表に無い天気コードは、絵文字無しでコードをそのまま出す。
pub fn render(date: NaiveDate, forecast: &DailyForecast, replacements: &[Replace]) -> String {
    let weather = match Weather::from_code(forecast.weather_code) {
        Some(weather) => format!("{} {}", emoji(weather.pattern), weather.name),
        None => format!("天気コード {}", forecast.weather_code),
    };
    let pops: Vec<String> = forecast.pops.iter().map(u8::to_string).collect();
    let min = forecast
        .min_temperature
        .map(|min| format!("↓{min} "))
        .unwrap_or_default();
    let body = format!(
        "{weather}\n降水確率: {} %\n気温: {min}↑{} ℃",
        pops.join("→"),
        forecast.max_temperature
    );
    format!(
        "{}\n{}",
        header(date, &forecast.area_name),
        apply_replacements(&body, replacements)
    )
}

/// 天気の絵文字の MFM。
fn emoji(pattern: Pattern) -> String {
    match pattern {
        Pattern::Simple(emoji) => format!("$[x2 {emoji}]"),
        Pattern::Occasional(main, sub) => format!("$[x2 {main}]$[position.y=0.3 {sub}]"),
        Pattern::Later(before, after) => format!("$[x2 {before}]→$[x2 {after}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forecast() -> DailyForecast {
        DailyForecast {
            area_name: "美濃地方".to_owned(),
            weather_code: 111,
            pops: vec![0, 0, 0, 10],
            min_temperature: Some(14),
            max_temperature: 22,
        }
    }

    fn date(value: &str) -> NaiveDate {
        value.parse().unwrap()
    }

    #[test]
    fn renders_the_header_weather_pops_and_temperatures() {
        assert_eq!(
            render(date("2026-10-23"), &forecast(), &[]),
            "10/23 天気予報(美濃地方)\n$[x2 ☀]→$[x2 ☁] 晴れ後曇り\n降水確率: 0→0→0→10 %\n気温: ↓14 ↑22 ℃"
        );
    }

    #[test]
    fn emoji_follows_the_pattern() {
        assert_eq!(emoji(Pattern::Simple("☀")), "$[x2 ☀]");
        assert_eq!(
            emoji(Pattern::Occasional("☁", "☀")),
            "$[x2 ☁]$[position.y=0.3 ☀]"
        );
        assert_eq!(emoji(Pattern::Later("☀", "☁")), "$[x2 ☀]→$[x2 ☁]");
    }

    #[test]
    fn today_has_no_minimum_and_unknown_codes_are_shown_as_is() {
        let forecast = DailyForecast {
            weather_code: 999,
            pops: vec![20],
            min_temperature: None,
            ..forecast()
        };
        assert_eq!(
            render(date("2026-10-23"), &forecast, &[]),
            "10/23 天気予報(美濃地方)\n天気コード 999\n降水確率: 20 %\n気温: ↑22 ℃"
        );
    }

    #[test]
    fn replacements_do_not_touch_the_header() {
        let replacements = [Replace {
            pattern: Regex::new(r"美濃地方|22 ℃").unwrap(),
            replacement: "X".to_owned(),
        }];
        assert_eq!(
            render(date("2026-10-23"), &forecast(), &replacements),
            "10/23 天気予報(美濃地方)\n$[x2 ☀]→$[x2 ☁] 晴れ後曇り\n降水確率: 0→0→0→10 %\n気温: ↓14 ↑X"
        );
    }

    #[test]
    fn the_header_identifies_the_note_and_its_date() {
        let text = render(date("2026-10-23"), &forecast(), &[]);
        assert!(is_mine(&text));
        assert!(is_for(&text, date("2026-10-23")));
        assert!(!is_for(&text, date("2026-10-24")));
        assert!(!is_mine("昨日(10/22)の気温(多治見)\n最高: 20 ℃ (14:00)"));
        assert!(!is_mine("天気予報(美濃地方)"));
    }
}
