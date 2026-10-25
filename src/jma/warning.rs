//! 警報・注意報（`bosai/warning/data/r8/{office}.json`）の解析。令和 8 年（2026-05-29）からの新体系。
//!
//! ファイルは配列で、電文種別（VPWW55 大雨・VPWW56 土砂災害・VPWW57 高潮・VPWW58 風雪・VPWW59 波浪・VPWW61 その他）
//! ごとに最新の 1 報が入る。各報の `warning.class20Items[]` が市区町村ごとの `kinds[]`（`{code, status}`）で、
//! 何も無い市区町村は `{status: "発表警報・注意報はなし"}` だけになる。発表中かどうかは「code があり status が 解除 でない」で
//! 判定する（status には 発表・継続のほか 警報から注意報 などの格下げの文言がある）。
//!
//! 種別のコード表は気象庁のサイト（`https://www.jma.go.jp/bosai/warning/` のスクリプト）の定義に合わせている。
//! 氾濫（旧 洪水）は河川ごとの別ファイルで出るので、ここでは扱わない。

use serde::Deserialize;
use tracing::warn;

/// 発表元のパス。
pub fn path(office: &str) -> String {
    format!("/bosai/warning/data/r8/{office}.json")
}

/// 警戒レベルに対応する段階。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 注意報（レベル 2）。
    Advisory,
    /// 警報（レベル 3）。
    Warning,
    /// 危険警報（レベル 4）。
    Urgent,
    /// 特別警報（レベル 5）。
    Emergency,
}

impl Level {
    /// 種別名の後ろに付ける語（`大雨` + `警報`）。
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Advisory => "注意報",
            Self::Warning => "警報",
            Self::Urgent => "危険警報",
            Self::Emergency => "特別警報",
        }
    }
}

/// 警報・注意報の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    /// 電文のコード（`03`）。
    pub code: &'static str,
    /// 現象の名前（`大雨`）。`suffix` と合わせて `大雨警報` になる。風は段階で名前が違う（強風注意報・暴風警報）。
    pub name: &'static str,
    pub level: Level,
}

use Level::{Advisory, Emergency, Urgent, Warning};

macro_rules! kind {
    ($code:literal, $name:literal, $level:expr) => {
        Kind {
            code: $code,
            name: $name,
            level: $level,
        }
    };
}

/// 全ての種別。ノートに並べる順でもある（レベルで分けた後、この順）。
pub const TABLE: &[Kind] = &[
    kind!("33", "大雨", Emergency),
    kind!("43", "大雨", Urgent),
    kind!("03", "大雨", Warning),
    kind!("10", "大雨", Advisory),
    kind!("39", "土砂災害", Emergency),
    kind!("49", "土砂災害", Urgent),
    kind!("09", "土砂災害", Warning),
    kind!("29", "土砂災害", Advisory),
    kind!("38", "高潮", Emergency),
    kind!("48", "高潮", Urgent),
    kind!("08", "高潮", Warning),
    kind!("19", "高潮", Advisory),
    kind!("35", "暴風", Emergency),
    kind!("05", "暴風", Warning),
    kind!("15", "強風", Advisory),
    kind!("32", "暴風雪", Emergency),
    kind!("02", "暴風雪", Warning),
    kind!("13", "風雪", Advisory),
    kind!("36", "大雪", Emergency),
    kind!("06", "大雪", Warning),
    kind!("12", "大雪", Advisory),
    kind!("37", "波浪", Emergency),
    kind!("07", "波浪", Warning),
    kind!("16", "波浪", Advisory),
    kind!("14", "雷", Advisory),
    kind!("17", "融雪", Advisory),
    kind!("20", "濃霧", Advisory),
    kind!("21", "乾燥", Advisory),
    kind!("22", "なだれ", Advisory),
    kind!("23", "低温", Advisory),
    kind!("24", "霜", Advisory),
    kind!("25", "着氷", Advisory),
    kind!("26", "着雪", Advisory),
];

impl Kind {
    /// `code` の種別。表に無ければ `None`。
    pub fn from_code(code: &str) -> Option<Self> {
        TABLE.iter().copied().find(|kind| kind.code == code)
    }
}

/// 発表の JSON。使うところだけ。
pub type Reports = Vec<Report>;

#[derive(Debug, Clone, Deserialize)]
pub struct Report {
    #[serde(default)]
    warning: Section,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Section {
    #[serde(default)]
    class20_items: Vec<AreaItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AreaItem {
    area_code: String,
    kinds: Vec<RawKind>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawKind {
    code: Option<String>,
    status: String,
}

/// 解除を表す status。
const CANCELLED: &str = "解除";

/// `area` が発表に載っている市区町村か。市区町村は何も発表されていなくても各報に載るので、載っていなければ、コードの誤りか、
/// 別の気象台の市区町村。
pub fn covers(reports: &Reports, area: &str) -> bool {
    reports
        .iter()
        .flat_map(|report| &report.warning.class20_items)
        .any(|item| item.area_code == area)
}

/// `area`（市区町村）に発表中の種別。[`TABLE`] の順で、重複は無い。知らないコードは警告して飛ばす。
pub fn active(reports: &Reports, area: &str) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = reports
        .iter()
        .flat_map(|report| &report.warning.class20_items)
        .filter(|item| item.area_code == area)
        .flat_map(|item| &item.kinds)
        .filter(|kind| kind.status != CANCELLED)
        .filter_map(|kind| {
            let code = kind.code.as_deref()?;
            let known = Kind::from_code(code);
            if known.is_none() {
                warn!(
                    code,
                    status = kind.status,
                    "skipping an unknown warning code"
                );
            }
            known
        })
        .collect();
    kinds.sort_by_key(|kind| TABLE.iter().position(|k| k == kind));
    kinds.dedup();
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEARED: &str = include_str!("../../tests/fixtures/jma/warning_210000.json");
    const YAEYAMA: &str = include_str!("../../tests/fixtures/jma/warning_474000.json");
    const KAGOSHIMA: &str = include_str!("../../tests/fixtures/jma/warning_460100.json");

    fn reports(json: &str) -> Reports {
        serde_json::from_str(json).unwrap()
    }

    fn codes(kinds: &[Kind]) -> Vec<&'static str> {
        kinds.iter().map(|kind| kind.code).collect()
    }

    #[test]
    fn the_table_has_unique_codes_and_the_expected_names() {
        let mut codes: Vec<&str> = TABLE.iter().map(|kind| kind.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len());

        let heavy_rain = Kind::from_code("43").unwrap();
        assert_eq!((heavy_rain.name, heavy_rain.level), ("大雨", Urgent));
        assert_eq!(heavy_rain.level.suffix(), "危険警報");
        let gale = Kind::from_code("15").unwrap();
        assert_eq!((gale.name, gale.level.suffix()), ("強風", "注意報"));
        // 旧体系の洪水と 19+ は無い。
        assert!(Kind::from_code("04").is_none());
        assert!(Kind::from_code("18").is_none());
        assert!(Kind::from_code("19+").is_none());
    }

    #[test]
    fn active_kinds_are_collected_across_reports_in_table_order() {
        // 石垣市: 大雨注意報・土砂災害注意報・高潮警報・暴風警報（継続）・波浪警報（継続）・雷注意報。
        let kinds = active(&reports(YAEYAMA), "4738100");
        assert_eq!(codes(&kinds), ["10", "29", "08", "05", "07", "14"]);
    }

    #[test]
    fn cancelled_kinds_and_empty_areas_are_not_active() {
        // 大雨注意報と土砂災害注意報は解除、波浪注意報は継続、雷注意報は発表。
        let kinds = active(&reports(KAGOSHIMA), "4650500");
        assert_eq!(codes(&kinds), ["16", "14"]);
        // 全て解除。
        assert!(active(&reports(CLEARED), "2120400").is_empty());
        assert!(active(&reports(YAEYAMA), "9999999").is_empty());
    }

    #[test]
    fn downgrades_are_still_active_and_duplicates_and_unknown_codes_are_dropped() {
        let reports: Reports = serde_json::from_str(
            r#"[
                {"warning": {"class20Items": [{"areaCode": "1", "kinds": [
                    {"code": "10", "status": "警報から注意報"},
                    {"code": "29", "status": "危険警報から注意報"},
                    {"code": "99", "status": "発表"}
                ]}]}},
                {"warning": {"class20Items": [{"areaCode": "1", "kinds": [
                    {"code": "10", "status": "継続"}
                ]}]}},
                {"warning": {}},
                {}
            ]"#,
        )
        .unwrap();
        assert_eq!(codes(&active(&reports, "1")), ["10", "29"]);
    }
}
