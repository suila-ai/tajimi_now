//! アメダス。地点表（`amedastable.json`）と、地点ごとの 3 時間ファイル（`{yyyyMMdd}_{HH}.json`）の解析。
//!
//! 3 時間ファイルのキーは `20261022210000` のような JST の時刻（10 分刻み）で、
//! 各項目は `[値, 品質フラグ]` の対。気象庁のアメダスのページに合わせて、品質フラグ 5 は値があっても休止中、2・3・6 は値があっても
//! 欠測（ページは `#`・`×` を出す）、それ以外で値が null なら欠測とする。1・4 はページが値に印を付けて出すので、値として扱う。
//! その地点で観測していない項目や、データの無い時刻は、項目ごと無い。
//! `maxTemp`・`minTemp` は、気象庁の日界（24 時）で区切ったその日の、その枠までの最高・最低気温。00:00 の枠には前日 1 日分が載る。
//! 起時（`maxTempTime`・`minTempTime`）は UTC の時・分で、JST に直す。日付は持たないので、日界の 24:00 は 00:00 になる。

use std::collections::HashMap;

use chrono::{
    DateTime, DurationRound, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike,
};
use serde::Deserialize;

use crate::time::JST;

/// 地点表のパス。
pub const STATIONS_PATH: &str = "/bosai/amedas/const/amedastable.json";

/// 地点表の 1 件のうち使うところ。
#[derive(Debug, Clone, Deserialize)]
pub struct Station {
    /// 漢字の地点名（`多治見`）。
    #[serde(rename = "kjName")]
    pub name: String,
}

/// 3 時間ファイルの中身。キーは時刻。
pub type Block = HashMap<String, Record>;

/// 10 分ごとの 1 件。`[値, 品質フラグ]` の対で、値は null になり得る。
#[derive(Debug, Clone, Deserialize)]
pub struct Record {
    temp: Option<Vec<Option<f64>>>,
    #[serde(rename = "precipitation1h")]
    precipitation_1h: Option<Vec<Option<f64>>>,
    #[serde(rename = "sun1h")]
    sun_1h: Option<Vec<Option<f64>>>,
    wind: Option<Vec<Option<f64>>>,
    #[serde(rename = "maxTemp")]
    max_temp: Option<Vec<Option<f64>>>,
    #[serde(rename = "maxTempTime")]
    max_temp_time: Option<ClockTime>,
    #[serde(rename = "minTemp")]
    min_temp: Option<Vec<Option<f64>>>,
    #[serde(rename = "minTempTime")]
    min_temp_time: Option<ClockTime>,
}

/// 起時。UTC の時・分。欠測の枠では時・分が null になる。
#[derive(Debug, Clone, Copy, Deserialize)]
struct ClockTime {
    hour: Option<u32>,
    minute: Option<u32>,
}

/// 10 分ごとの観測値。
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// 観測時刻（JST）。
    pub time: NaiveDateTime,
    /// 気温（℃）。
    pub temperature: Reading,
    /// 前 1 時間の降水量（mm）。
    pub precipitation_1h: Reading,
    /// 前 1 時間の日照時間（分）。
    pub sunshine_minutes: Reading,
    /// 風速（m/s）。
    pub wind_speed: Reading,
    /// この枠が属する日の、この枠までの最高気温。
    pub max_temperature: Option<Extreme>,
    /// この枠が属する日の、この枠までの最低気温。
    pub min_temperature: Option<Extreme>,
}

/// 最高か最低の気温と、その起時（JST の時刻）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extreme {
    /// 気温（℃）。
    pub temperature: f64,
    pub time: NaiveTime,
}

/// 10 分値の 1 項目。値が無いときは、気象庁のアメダスのページの表示と同じく分ける。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    Value(f64),
    /// 品質フラグが 5。
    Suspended,
    /// 値が null か、気象庁のページが値を出さない品質フラグ（2・3・6）。
    Missing,
    /// 項目が無い。その地点で観測していないか、その時刻のデータが無い。
    Absent,
}

impl Reading {
    fn map(self, f: impl FnOnce(f64) -> f64) -> Self {
        match self {
            Self::Value(value) => Self::Value(f(value)),
            other => other,
        }
    }
}

/// 10 分枠の長さ。
pub const SLOT: TimeDelta = TimeDelta::minutes(10);

/// `at` を含む 10 分枠の始まり（JST）。
pub fn slot(at: DateTime<FixedOffset>) -> NaiveDateTime {
    at.with_timezone(&JST)
        .duration_trunc(SLOT)
        .expect("10 minutes divides a day")
        .naive_local()
}

/// `time`（JST）を含む 3 時間ファイルのパス。
pub fn block_path(point: &str, time: NaiveDateTime) -> String {
    format!(
        "/bosai/amedas/data/point/{point}/{}_{:02}.json",
        time.format("%Y%m%d"),
        time.hour() / 3 * 3
    )
}

/// `date` の最後の 10 分枠。気象庁の日界は 24 時なので翌日の 00:00 で、この枠のレコードにその日の最高・最低が載る。
pub fn day_end(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN) + TimeDelta::days(1)
}

/// 3 時間ファイルから、枠 `slot` の観測値を取り出す。その枠がまだ無ければ `None`。
pub fn observation(block: &Block, slot: NaiveDateTime) -> Option<Observation> {
    let record = block.get(&slot.format("%Y%m%d%H%M%S").to_string())?;
    Some(Observation {
        time: slot,
        temperature: reading(&record.temp),
        precipitation_1h: reading(&record.precipitation_1h),
        sunshine_minutes: reading(&record.sun_1h).map(|hours| hours * 60.0),
        wind_speed: reading(&record.wind),
        max_temperature: extreme(&record.max_temp, record.max_temp_time),
        min_temperature: extreme(&record.min_temp, record.min_temp_time),
    })
}

/// 休止中を表す品質フラグ。
const SUSPENDED: f64 = 5.0;
/// 気象庁のアメダスのページが値を出さない品質フラグ。2・3 は `#`、5 は休止中、6 は `×` を出す。
const WITHHELD: [f64; 4] = [2.0, 3.0, SUSPENDED, 6.0];

/// `[値, 品質フラグ]` の値。値が null か、品質フラグが [`WITHHELD`] なら `None`。
fn value(pair: &[Option<f64>]) -> Option<f64> {
    let withheld = pair
        .get(1)
        .copied()
        .flatten()
        .is_some_and(|flag| WITHHELD.contains(&flag));
    pair.first().copied().flatten().filter(|_| !withheld)
}

/// `[値, 品質フラグ]` の項目。
fn reading(pair: &Option<Vec<Option<f64>>>) -> Reading {
    let Some(pair) = pair else {
        return Reading::Absent;
    };
    if pair.get(1).copied().flatten() == Some(SUSPENDED) {
        Reading::Suspended
    } else {
        value(pair).map_or(Reading::Missing, Reading::Value)
    }
}

/// 最高か最低。値を出さない品質フラグなら無いものとする。起時は UTC の時・分なので、JST の時刻に直す。日付は持たないので、
/// 日界の 24:00 は 00:00 になる。
fn extreme(pair: &Option<Vec<Option<f64>>>, at: Option<ClockTime>) -> Option<Extreme> {
    let temperature = value(pair.as_deref()?)?;
    let at = at?;
    let utc = NaiveTime::from_hms_opt(at.hour?, at.minute?, 0)?;
    Some(Extreme {
        temperature,
        time: utc + TimeDelta::seconds(i64::from(JST.local_minus_utc())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATIONS: &str = include_str!("../../tests/fixtures/jma/amedastable.json");
    const BLOCK_21: &str = include_str!("../../tests/fixtures/jma/amedas_52606_20261022_21.json");
    const BLOCK_00: &str = include_str!("../../tests/fixtures/jma/amedas_52606_20261021_00.json");

    fn naive(value: &str) -> NaiveDateTime {
        value.parse().unwrap()
    }

    fn extreme(temperature: f64, time: &str) -> Option<Extreme> {
        Some(Extreme {
            temperature,
            time: time.parse().unwrap(),
        })
    }

    #[test]
    fn stations_are_read_by_code() {
        let stations: HashMap<String, Station> = serde_json::from_str(STATIONS).unwrap();
        assert_eq!(stations["52606"].name, "多治見");
        assert_eq!(stations["11001"].name, "宗谷岬");
    }

    #[test]
    fn a_slot_is_read_by_its_key() {
        let block: Block = serde_json::from_str(BLOCK_21).unwrap();
        assert_eq!(
            observation(&block, naive("2026-10-22T21:00:00")),
            Some(Observation {
                time: naive("2026-10-22T21:00:00"),
                temperature: Reading::Value(14.3),
                precipitation_1h: Reading::Value(0.0),
                sunshine_minutes: Reading::Value(0.0),
                wind_speed: Reading::Value(0.8),
                max_temperature: extreme(24.0, "14:54:00"),
                min_temperature: extreme(14.1, "21:00:00"),
            })
        );
        // ファイルの最後の枠は 22:10 で、その次の枠はまだ無い。
        assert!(observation(&block, naive("2026-10-22T22:10:00")).is_some());
        assert!(observation(&block, naive("2026-10-22T22:20:00")).is_none());
    }

    #[test]
    fn the_00_00_record_carries_the_previous_days_extremes() {
        let block: Block = serde_json::from_str(BLOCK_00).unwrap();
        // 00:00 の枠は 10/20 の 24:00 なので、10/20 の 1 日分（気象庁の確定値と同じ）。
        let midnight = observation(&block, naive("2026-10-21T00:00:00")).unwrap();
        assert_eq!(midnight.max_temperature, extreme(26.8, "15:18:00"));
        assert_eq!(midnight.min_temperature, extreme(19.4, "01:42:00"));
        // 00:10 の枠から 10/21 の集計になる。
        let next = observation(&block, naive("2026-10-21T00:10:00")).unwrap();
        assert_eq!(next.max_temperature, extreme(23.6, "00:10:00"));
        assert_eq!(next.min_temperature, extreme(23.4, "00:08:00"));
    }

    #[test]
    fn an_extreme_at_15_00_utc_is_00_00() {
        let block: Block = serde_json::from_str(
            r#"{"20261023000000": {"minTemp": [1.0, 0], "minTempTime": {"hour": 15, "minute": 0}}}"#,
        )
        .unwrap();
        let midnight = observation(&block, naive("2026-10-23T00:00:00")).unwrap();
        assert_eq!(midnight.min_temperature, extreme(1.0, "00:00:00"));
        assert_eq!(midnight.max_temperature, None);
    }

    #[test]
    fn a_missing_slot_leaves_the_rest_of_the_block_readable() {
        // 欠測の枠は起時の時・分も null になる（雄和 32451 の 2026-10-23 00:00）。
        let block: Block = serde_json::from_str(
            r#"{
                "20261023000000": {"maxTempTime": {"hour": null, "minute": null}, "minTempTime": {"hour": null, "minute": null}},
                "20261023001000": {"temp": [8.2, 0], "maxTemp": [8.6, 4], "maxTempTime": {"hour": 15, "minute": 8}}
            }"#,
        )
        .unwrap();
        let missing = observation(&block, naive("2026-10-23T00:00:00")).unwrap();
        assert_eq!(missing.max_temperature, None);
        let next = observation(&block, naive("2026-10-23T00:10:00")).unwrap();
        assert_eq!(next.temperature, Reading::Value(8.2));
        assert_eq!(next.max_temperature, extreme(8.6, "00:08:00"));
    }

    #[test]
    fn values_without_a_number_follow_the_quality_flag() {
        let block: Block = serde_json::from_str(
            r#"{
                "20261022210000": {"temp": [null, 6], "precipitation1h": [0.5, 0], "wind": [null, 5], "sun1h": [0.4, 5]},
                "20261022211000": {"humidity": [60, 0]}
            }"#,
        )
        .unwrap();
        let first = observation(&block, naive("2026-10-22T21:00:00")).unwrap();
        assert_eq!(first.temperature, Reading::Missing);
        assert_eq!(first.precipitation_1h, Reading::Value(0.5));
        assert_eq!(first.wind_speed, Reading::Suspended);
        // 値があっても、品質フラグ 5 なら休止中。
        assert_eq!(first.sunshine_minutes, Reading::Suspended);
        assert_eq!(
            observation(&block, naive("2026-10-22T21:10:00")),
            Some(Observation {
                time: naive("2026-10-22T21:10:00"),
                temperature: Reading::Absent,
                precipitation_1h: Reading::Absent,
                sunshine_minutes: Reading::Absent,
                wind_speed: Reading::Absent,
                max_temperature: None,
                min_temperature: None,
            })
        );
    }

    #[test]
    fn values_the_jma_page_withholds_are_missing() {
        let block: Block = serde_json::from_str(
            r#"{"20261024000000": {
                "temp": [8.2, 2], "wind": [1.5, 6], "precipitation1h": [0.0, 4],
                "maxTemp": [16.5, 3], "maxTempTime": {"hour": 5, "minute": 36},
                "minTemp": [7.5, 1], "minTempTime": {"hour": 19, "minute": 15}
            }}"#,
        )
        .unwrap();
        let midnight = observation(&block, naive("2026-10-24T00:00:00")).unwrap();
        assert_eq!(midnight.temperature, Reading::Missing);
        assert_eq!(midnight.wind_speed, Reading::Missing);
        assert_eq!(midnight.max_temperature, None);
        // 1・4 はページも値を出す。
        assert_eq!(midnight.precipitation_1h, Reading::Value(0.0));
        assert_eq!(midnight.min_temperature, extreme(7.5, "04:15:00"));
    }

    #[test]
    fn sunshine_is_converted_from_hours_to_minutes() {
        let block: Block =
            serde_json::from_str(r#"{"20261022120000": {"sun1h": [0.5, 0]}}"#).unwrap();
        let noon = observation(&block, naive("2026-10-22T12:00:00")).unwrap();
        assert_eq!(noon.sunshine_minutes, Reading::Value(30.0));
    }

    #[test]
    fn slot_truncates_to_ten_minutes_in_jst() {
        let at = |value: &str| DateTime::parse_from_rfc3339(value).unwrap();
        assert_eq!(
            slot(at("2026-10-22T21:17:45+09:00")),
            naive("2026-10-22T21:10:00")
        );
        assert_eq!(
            slot(at("2026-10-22T21:00:00+09:00")),
            naive("2026-10-22T21:00:00")
        );
        // UTC の時刻も JST に直してから切り下げる。
        assert_eq!(
            slot(at("2026-10-22T14:59:59Z")),
            naive("2026-10-22T23:50:00")
        );
    }

    #[test]
    fn block_path_uses_three_hour_files() {
        assert_eq!(
            block_path("52606", naive("2026-10-22T21:17:00")),
            "/bosai/amedas/data/point/52606/20261022_21.json"
        );
        assert_eq!(
            block_path("52606", naive("2026-10-22T02:50:00")),
            "/bosai/amedas/data/point/52606/20261022_00.json"
        );
        assert_eq!(
            block_path("52606", naive("2026-10-22T23:59:00")),
            "/bosai/amedas/data/point/52606/20261022_21.json"
        );
    }

    #[test]
    fn day_end_is_the_next_midnight() {
        assert_eq!(
            day_end(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap()),
            naive("2026-10-23T00:00:00")
        );
    }
}
