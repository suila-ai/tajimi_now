//! 天気予報の天気コード（`100` 晴れ など）の名称と絵文字。旧 TajimiNow の表をそのまま使う。

/// 絵文字の並べ方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// 1 つの天気。
    Simple(&'static str),
    /// 「時々」「一時」など。主の天気の脇に小さく添える。
    Occasional(&'static str, &'static str),
    /// 「後」など。矢印でつなぐ。
    Later(&'static str, &'static str),
}

use Pattern::{Later, Occasional, Simple};

/// コード → (名称, 絵文字)。
const TABLE: &[(u16, &str, Pattern)] = &[
    (100, "晴れ", Simple("☀")),
    (101, "晴れ時々曇り", Occasional("☀", "☁")),
    (102, "晴れ一時雨", Occasional("☀", "☂")),
    (103, "晴れ時々雨", Occasional("☀", "☂")),
    (104, "晴れ一時雪", Occasional("☀", "❄")),
    (105, "晴れ時々雪", Occasional("☀", "❄")),
    (106, "晴れ一時雨か雪", Occasional("☀", "☂❄")),
    (107, "晴れ時々雨か雪", Occasional("☀", "☂❄")),
    (108, "晴れ一時雨か雷雨", Occasional("☀", "☂⚡")),
    (110, "晴れ後時々曇り", Later("☀", "☁")),
    (111, "晴れ後曇り", Later("☀", "☁")),
    (112, "晴れ後一時雨", Later("☀", "☂")),
    (113, "晴れ後時々雨", Later("☀", "☂")),
    (114, "晴れ後雨", Later("☀", "☂")),
    (115, "晴れ後一時雪", Later("☀", "❄")),
    (116, "晴れ後時々雪", Later("☀", "❄")),
    (117, "晴れ後雪", Later("☀", "❄")),
    (118, "晴れ後雨か雪", Later("☀", "☂❄")),
    (119, "晴れ後雨か雷雨", Later("☀", "☂⚡")),
    (120, "晴れ朝夕一時雨", Occasional("☀", "☂")),
    (121, "晴れ朝の内一時雨", Occasional("☀", "☂")),
    (122, "晴れ夕方一時雨", Later("☀", "☂")),
    (123, "晴れ山沿い雷雨", Occasional("☀", "☂⚡")),
    (124, "晴れ山沿い雪", Occasional("☀", "❄")),
    (125, "晴れ午後は雷雨", Later("☀", "☂⚡")),
    (126, "晴れ昼頃から雨", Later("☀", "☂")),
    (127, "晴れ夕方から雨", Later("☀", "☂")),
    (128, "晴れ夜は雨", Later("☀", "☂")),
    (130, "朝の内霧後晴れ", Simple("☀")),
    (131, "晴れ明け方霧", Simple("☀")),
    (132, "晴れ朝夕曇り", Occasional("☀", "☁")),
    (140, "晴れ時々雨で雷を伴う", Occasional("☀", "☂⚡")),
    (160, "晴れ一時雪か雨", Occasional("☀", "❄☂")),
    (170, "晴れ時々雪か雨", Occasional("☀", "❄☂")),
    (181, "晴れ後雪か雨", Later("☀", "❄☂")),
    (200, "曇り", Simple("☁")),
    (201, "曇り時々晴れ", Occasional("☁", "☀")),
    (202, "曇り一時雨", Occasional("☁", "☂")),
    (203, "曇り時々雨", Occasional("☁", "☂")),
    (204, "曇り一時雪", Occasional("☁", "❄")),
    (205, "曇り時々雪", Occasional("☁", "❄")),
    (206, "曇り一時雨か雪", Occasional("☁", "☂❄")),
    (207, "曇り時々雨か雪", Occasional("☁", "☂❄")),
    (208, "曇り一時雨か雷雨", Occasional("☁", "☂⚡")),
    (209, "霧", Simple("☁")),
    (210, "曇り後時々晴れ", Later("☁", "☀")),
    (211, "曇り後晴れ", Later("☁", "☀")),
    (212, "曇り後一時雨", Later("☁", "☂")),
    (213, "曇り後時々雨", Later("☁", "☂")),
    (214, "曇り後雨", Later("☁", "☂")),
    (215, "曇り後一時雪", Later("☁", "❄")),
    (216, "曇り後時々雪", Later("☁", "❄")),
    (217, "曇り後雪", Later("☁", "❄")),
    (218, "曇り後雨か雪", Later("☁", "☂❄")),
    (219, "曇り後雨か雷雨", Later("☁", "☂⚡")),
    (220, "曇り朝夕一時雨", Occasional("☁", "☂")),
    (221, "曇り朝の内一時雨", Occasional("☁", "☂")),
    (222, "曇り夕方一時雨", Later("☁", "☂")),
    (223, "曇り日中時々晴れ", Occasional("☁", "☀")),
    (224, "曇り昼頃から雨", Later("☁", "☂")),
    (225, "曇り夕方から雨", Later("☁", "☂")),
    (226, "曇り夜は雨", Later("☁", "☂")),
    (228, "曇り昼頃から雪", Later("☁", "❄")),
    (229, "曇り夕方から雪", Later("☁", "❄")),
    (230, "曇り夜は雪", Later("☁", "❄")),
    (231, "曇り海上海岸は霧か霧雨", Occasional("☁", "☂")),
    (240, "曇り時々雨で雷を伴う", Occasional("☁", "☂⚡")),
    (250, "曇り時々雪で雷を伴う", Occasional("☁", "❄⚡")),
    (260, "曇り一時雪か雨", Occasional("☁", "❄☂")),
    (270, "曇り時々雪か雨", Occasional("☁", "❄☂")),
    (281, "曇り後雪か雨", Later("☁", "❄☂")),
    (300, "雨", Simple("☂")),
    (301, "雨時々晴れ", Occasional("☂", "☀")),
    (302, "雨時々止む", Occasional("☂", "☁")),
    (303, "雨時々雪", Occasional("☂", "❄")),
    (304, "雨か雪", Simple("☂❄")),
    (306, "大雨", Simple("☂")),
    (308, "雨で暴風を伴う", Simple("☂💨")),
    (309, "雨一時雪", Occasional("☂", "❄")),
    (311, "雨後晴れ", Later("☂", "☀")),
    (313, "雨後曇り", Later("☂", "☁")),
    (314, "雨後時々雪", Later("☂", "❄")),
    (315, "雨後雪", Later("☂", "❄")),
    (316, "雨か雪後晴れ", Later("☂❄", "☀")),
    (317, "雨か雪後曇り", Later("☂❄", "☁")),
    (320, "朝の内雨後晴れ", Later("☂", "☀")),
    (321, "朝の内雨後曇り", Later("☂", "☁")),
    (322, "雨朝晩一時雪", Occasional("☂", "❄")),
    (323, "雨昼頃から晴れ", Later("☂", "☀")),
    (324, "雨夕方から晴れ", Later("☂", "☀")),
    (325, "雨夜は晴れ", Later("☂", "☀")),
    (326, "雨夕方から雪", Later("☂", "❄")),
    (327, "雨夜は雪", Later("☂", "❄")),
    (328, "雨一時強く降る", Simple("☂")),
    (329, "雨一時みぞれ", Simple("☂")),
    (340, "雪か雨", Simple("❄☂")),
    (350, "雨で雷を伴う", Simple("☂⚡")),
    (361, "雪か雨後晴れ", Later("❄☂", "☀")),
    (371, "雪か雨後曇り", Later("❄☂", "☁")),
    (400, "雪", Simple("❄")),
    (401, "雪時々晴れ", Occasional("❄", "☀")),
    (402, "雪時々止む", Occasional("❄", "☁")),
    (403, "雪時々雨", Occasional("❄", "☂")),
    (405, "大雪", Simple("❄")),
    (406, "風雪強い", Simple("❄💨")),
    (407, "暴風雪", Simple("❄💨")),
    (409, "雪一時雨", Occasional("❄", "☂")),
    (411, "雪後晴れ", Later("❄", "☀")),
    (413, "雪後曇り", Later("❄", "☁")),
    (414, "雪後雨", Later("❄", "☂")),
    (420, "朝の内雪後晴れ", Later("❄", "☀")),
    (421, "朝の内雪後曇り", Later("❄", "☁")),
    (422, "雪昼頃から雨", Later("❄", "☂")),
    (423, "雪夕方から雨", Later("❄", "☂")),
    (425, "雪一時強く降る", Simple("❄")),
    (426, "雪後みぞれ", Simple("❄")),
    (427, "雪一時みぞれ", Simple("❄")),
    (450, "雪で雷を伴う", Simple("❄⚡")),
];

/// 天気コードの名称と絵文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    pub name: &'static str,
    pub pattern: Pattern,
}

impl Weather {
    /// `code` の天気。表に無ければ `None`。
    pub fn from_code(code: u16) -> Option<Self> {
        TABLE
            .iter()
            .find(|(c, _, _)| *c == code)
            .map(|&(_, name, pattern)| Self { name, pattern })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_known_codes_are_found() {
        let mut codes: Vec<u16> = TABLE.iter().map(|(code, _, _)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len());

        let sunny = Weather::from_code(100).unwrap();
        assert_eq!(sunny.name, "晴れ");
        assert_eq!(sunny.pattern, Simple("☀"));
        let later = Weather::from_code(111).unwrap();
        assert_eq!(later.name, "晴れ後曇り");
        assert_eq!(later.pattern, Later("☀", "☁"));
        let occasional = Weather::from_code(201).unwrap();
        assert_eq!(occasional.name, "曇り時々晴れ");
        assert_eq!(occasional.pattern, Occasional("☁", "☀"));
        assert!(Weather::from_code(999).is_none());
    }
}
