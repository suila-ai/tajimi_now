//! 天気予報（`bosai/forecast/data/forecast/{office}.json`）の解析。
//!
//! 配列の先頭が明後日までの予報で、`timeSeries[0]` が天気（一次細分区域ごと）、`[1]` が降水確率、`[2]` が気温
//! （区域の代表アメダス地点ごと）。気温の地点は区域と数も並びも合わない気象台があるので、予報区域の対応表
//! （`bosai/forecast/const/forecast_area.json`）で引く。
//! 05 時・11 時の発表では、気温の時刻が `[今日09:00, 今日00:00, 明日00:00, 明日09:00]` と並び、
//! 今日 00:00 の枠には最高気温と同じ値が入る（埋め草）。00:00 の枠が同じ日の 09:00 の枠より後ろにあれば、最低気温は無いとみなす。

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;

/// 発表元のパス。
pub fn path(office: &str) -> String {
    format!("/bosai/forecast/data/forecast/{office}.json")
}

/// 予報区域の対応表のパス。
pub const AREAS_PATH: &str = "/bosai/forecast/const/forecast_area.json";

/// 気象台ごとに、一次細分区域と気温の地点を並べたもの。
pub type AreaTable = HashMap<String, Vec<AreaLink>>;

/// 対応表の 1 件のうち使うところ。
#[derive(Debug, Clone, Deserialize)]
pub struct AreaLink {
    /// 一次細分区域のコード。
    class10: String,
    /// 区域の気温の地点のコード。
    amedas: Vec<String>,
}

/// 1 つの区域の、1 日分の予報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyForecast {
    /// 区域名（`美濃地方`）。
    pub area_name: String,
    /// 天気コード（`111`）。
    pub weather_code: u16,
    /// 降水確率（%）。その日の時間帯の順。発表時刻より前の時間帯は入っていない。
    pub pops: Vec<u8>,
    /// 最低気温（℃）。当日の発表（05 時・11 時）には無い。
    pub min_temperature: Option<i32>,
    /// 最高気温（℃）。
    pub max_temperature: i32,
}

/// 発表の JSON。使うところだけ。
pub type Reports = Vec<Report>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    time_series: Vec<TimeSeries>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeSeries {
    time_defines: Vec<DateTime<FixedOffset>>,
    areas: Vec<Area>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Area {
    area: AreaInfo,
    #[serde(default)]
    weather_codes: Vec<String>,
    #[serde(default)]
    pops: Vec<String>,
    #[serde(default)]
    temps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AreaInfo {
    name: String,
    code: String,
}

/// `area` が発表に載っている区域か。載っていなければ、区域のコードの誤りか、別の気象台の区域。
pub fn covers(reports: &Reports, area: &str) -> bool {
    reports
        .first()
        .and_then(|report| report.time_series.first())
        .is_some_and(|weather| weather.areas.iter().any(|a| a.area.code == area))
}

/// `area`（一次細分区域）の `date` の予報。天気・降水確率・最高気温がそろっていなければ `None`。
pub fn daily(
    reports: &Reports,
    table: &AreaTable,
    area: &str,
    date: NaiveDate,
) -> Option<DailyForecast> {
    let series = &reports.first()?.time_series;
    let (weather, pops, temps) = (series.first()?, series.get(1)?, series.get(2)?);
    let weather_area = weather.areas.iter().find(|a| a.area.code == area)?;

    let weather_index = weather
        .time_defines
        .iter()
        .position(|time| time.date_naive() == date)?;
    let weather_code = weather_area
        .weather_codes
        .get(weather_index)?
        .parse()
        .ok()?;

    let pops_area = pops.areas.iter().find(|a| a.area.code == area)?;
    let pops: Vec<u8> = pops
        .time_defines
        .iter()
        .zip(&pops_area.pops)
        .filter(|(time, _)| time.date_naive() == date)
        .filter_map(|(_, pop)| pop.parse().ok())
        .collect();
    if pops.is_empty() {
        return None;
    }

    // 区域は別の気象台のキーに載ることがある（十勝は釧路の発表）ので、表全体から引く。
    let points = &table
        .values()
        .flatten()
        .find(|link| link.class10 == area)?
        .amedas;
    let temps_area = points
        .iter()
        .find_map(|point| temps.areas.iter().find(|a| &a.area.code == point))?;
    let temp_at = |hour: u32| {
        let target = date.and_hms_opt(hour, 0, 0)?;
        let position = temps
            .time_defines
            .iter()
            .position(|time| time.naive_local() == target)?;
        Some((
            position,
            temps_area.temps.get(position)?.parse::<i32>().ok()?,
        ))
    };
    let (max_position, max_temperature) = temp_at(9)?;
    let min_temperature = temp_at(0)
        .filter(|&(min_position, _)| min_position < max_position)
        .map(|(_, temperature)| temperature);

    Some(DailyForecast {
        area_name: weather_area.area.name.clone(),
        weather_code,
        pops,
        min_temperature,
        max_temperature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENING: &str = include_str!("../../tests/fixtures/jma/forecast_210000_1700.json");
    const MORNING: &str = include_str!("../../tests/fixtures/jma/forecast_130000_0500.json");
    const AREAS: &str = include_str!("../../tests/fixtures/jma/forecast_area.json");

    fn table() -> AreaTable {
        serde_json::from_str(AREAS).unwrap()
    }

    fn reports(json: &str) -> Reports {
        serde_json::from_str(json).unwrap()
    }

    fn date(value: &str) -> NaiveDate {
        value.parse().unwrap()
    }

    #[test]
    fn tomorrow_has_both_temperatures_in_the_evening_report() {
        let forecast = daily(&reports(EVENING), &table(), "210010", date("2026-10-23")).unwrap();
        assert_eq!(
            forecast,
            DailyForecast {
                area_name: "美濃地方".to_owned(),
                weather_code: 201,
                pops: vec![0, 0, 0, 10],
                min_temperature: Some(14),
                max_temperature: 22,
            }
        );
        // 飛騨地方の気温は対応表の地点（高山）から取る。
        let hida = daily(&reports(EVENING), &table(), "210020", date("2026-10-23")).unwrap();
        assert_eq!(hida.area_name, "飛騨地方");
        assert_eq!((hida.min_temperature, hida.max_temperature), (Some(7), 22));
    }

    #[test]
    fn today_has_no_minimum_in_the_morning_report() {
        let forecast = daily(&reports(MORNING), &table(), "130010", date("2026-10-16")).unwrap();
        assert_eq!(forecast.weather_code, 101);
        assert_eq!(forecast.pops, vec![0, 0, 0]);
        // 今日 00:00 の枠は 09:00 の枠より後ろにある埋め草（26）なので、最低気温にしない。
        assert_eq!(forecast.min_temperature, None);
        assert_eq!(forecast.max_temperature, 26);

        let tomorrow = daily(&reports(MORNING), &table(), "130010", date("2026-10-17")).unwrap();
        assert_eq!(tomorrow.pops, vec![0, 0, 0, 10]);
        assert_eq!(
            (tomorrow.min_temperature, tomorrow.max_temperature),
            (Some(17), 25)
        );
    }

    #[test]
    fn missing_days_areas_and_temperatures_are_none() {
        let evening = reports(EVENING);
        // 今日は天気はあるが気温の枠が無い。
        assert!(daily(&evening, &table(), "210010", date("2026-10-22")).is_none());
        // 明後日は気温が無い。
        assert!(daily(&evening, &table(), "210010", date("2026-10-24")).is_none());
        assert!(daily(&evening, &table(), "999999", date("2026-10-23")).is_none());
        assert!(daily(&Vec::new(), &table(), "210010", date("2026-10-23")).is_none());
        // 対応表に無い区域は気温が引けない。
        assert!(daily(&evening, &AreaTable::new(), "210010", date("2026-10-23")).is_none());
    }

    #[test]
    fn unparsable_temperatures_are_none() {
        let mut value: serde_json::Value = serde_json::from_str(EVENING).unwrap();
        let temps = &mut value[0]["timeSeries"][2]["areas"][0]["temps"];
        assert_eq!(temps[1], "22", "the fixture has the expected temps");
        temps[1] = serde_json::Value::String(String::new());
        let reports: Reports = serde_json::from_value(value).unwrap();
        assert!(daily(&reports, &table(), "210010", date("2026-10-23")).is_none());
    }

    #[test]
    fn temperatures_follow_the_area_table() {
        // 区域が別の気象台のキーにあっても引き、予報の気温に載っていない地点（99999）は飛ばす。
        let table: AreaTable = serde_json::from_str(
            r#"{"999999": [{"class10": "210010", "amedas": ["99999", "52146"], "class20": "2120100"}]}"#,
        )
        .unwrap();
        let forecast = daily(&reports(EVENING), &table, "210010", date("2026-10-23")).unwrap();
        assert_eq!(forecast.area_name, "美濃地方");
        assert_eq!(
            (forecast.min_temperature, forecast.max_temperature),
            (Some(7), 22)
        );
    }

    #[test]
    fn a_day_without_pops_is_none() {
        let mut value: serde_json::Value = serde_json::from_str(EVENING).unwrap();
        let pops = &mut value[0]["timeSeries"][1]["areas"][0]["pops"];
        for pop in pops.as_array_mut().unwrap() {
            *pop = serde_json::Value::String(String::new());
        }
        let reports: Reports = serde_json::from_value(value).unwrap();
        assert!(daily(&reports, &table(), "210010", date("2026-10-23")).is_none());
    }
}
