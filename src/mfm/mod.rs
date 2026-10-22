//! Misskey に投稿する本文（MFM）の組み立てと読み戻し。
//!
//! 本文は、1 行目のヘッダ（予報・前日気温）か最終行のフッタ（観測値）で、bot のどのノートかを見分けられる書式にしてある。
//! 設定の置換は、ヘッダとフッタを除いた本文にだけ当てる。

pub mod forecast;
pub mod min_max;
pub mod observation;

use crate::config::Replace;

/// 設定の置換を順に当てる。
pub fn apply_replacements(text: &str, replacements: &[Replace]) -> String {
    replacements.iter().fold(text.to_owned(), |text, replace| {
        replace
            .pattern
            .replace_all(&text, replace.replacement.as_str())
            .into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(pattern: &str, replacement: &str) -> Replace {
        Replace {
            pattern: regex::Regex::new(pattern).unwrap(),
            replacement: replacement.to_owned(),
        }
    }

    #[test]
    fn replacements_apply_in_order_to_every_match() {
        let replacements = [replace(r"33\.4 ℃", ":334:℃"), replace("℃", "度")];
        assert_eq!(
            apply_replacements("最高: 33.4 ℃\n最低: 33.4 ℃", &replacements),
            "最高: :334:度\n最低: :334:度"
        );
        assert_eq!(apply_replacements("そのまま", &[]), "そのまま");
    }
}
