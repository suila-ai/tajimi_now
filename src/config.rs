//! TOML の設定ファイル。起動時に 1 回だけ読む。
//!
//! 節は投稿の種類ごとにある。`[amedas]` は観測値と前日の気温、`[forecast]` は天気予報、`[warning]` は観測値に添える
//! 警報・注意報で、節を省くとその投稿をしない。`[chance]` は気温の閾値で添付するファイル、`[[replace]]` は本文の置換で、
//! どちらも任意。

use std::path::{Path, PathBuf};
use std::str::FromStr;

use regex::Regex;
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub misskey: MisskeyConfig,
    /// 観測値（10 分ごと）と前日の気温（日次）。
    pub amedas: Option<AmedasConfig>,
    /// 天気予報（日次）。
    pub forecast: Option<ForecastConfig>,
    /// 観測値に添える警報・注意報。
    pub warning: Option<WarningConfig>,
    /// 気温の閾値と、達したときに添付するファイル。
    #[serde(default)]
    pub chance: ChanceConfig,
    /// 本文に順に当てる正規表現の置換。
    #[serde(default)]
    pub replace: Vec<Replace>,
}

/// Misskey への接続。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MisskeyConfig {
    /// サーバのベース URL。
    pub server: Url,
    /// アクセストークン。`read:account`、`write:account`、`write:notes` の権限が要る。
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmedasConfig {
    /// 地点コード。`https://www.jma.go.jp/bosai/amedas/const/amedastable.json` のキー。
    pub point: String,
    /// 観測値のノートの公開範囲。
    #[serde(default)]
    pub observation_visibility: Visibility,
    /// 前日の気温のノートの公開範囲。
    #[serde(default)]
    pub min_max_visibility: Visibility,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastConfig {
    /// 予報を発表する気象台のコード（`/bosai/forecast/data/forecast/{office}.json`）。ふつうは
    /// `https://www.jma.go.jp/bosai/common/const/area.json` の `offices` のキーで、十勝（014030）は 014100、奄美（460040）は 460100。
    pub office: String,
    /// 一次細分区域のコード。`area.json` の `class10s` のキー。
    pub area: String,
    #[serde(default)]
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarningConfig {
    /// 気象台のコード。`area.json` の `offices` のキー。
    pub office: String,
    /// 市区町村のコード。`area.json` の `class20s` のキー。
    pub area: String,
}

/// 気温の閾値。予報か観測の最高気温が `max_threshold` 以上なら、最低気温が `min_threshold` 以下なら、対応するファイルを添付する。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChanceConfig {
    pub max_threshold: Option<f64>,
    /// 予報で達したときに添付するドライブのファイル ID。
    pub max_forecast_file: Option<String>,
    /// 観測で達したときに添付するドライブのファイル ID。
    pub max_achieved_file: Option<String>,
    pub min_threshold: Option<f64>,
    pub min_forecast_file: Option<String>,
    pub min_achieved_file: Option<String>,
}

/// 本文への正規表現の置換。`replacement` には `$1` や `${name}` のような参照を書ける。
/// 参照の直後に英数字が続くなら `${1}mm` のように波括弧で囲む（`$1mm` は `1mm` という名前の参照になり、空になる）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replace {
    #[serde(deserialize_with = "regex")]
    pub pattern: Regex,
    pub replacement: String,
}

fn regex<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Regex, D::Error> {
    Regex::new(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

/// ノートの公開範囲。設定には小文字で書く。
///
/// `Specified` は宛先を付けずに送り、自分だけが読めるノートにする（デバッグ用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    #[default]
    Home,
    Followers,
    Specified,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read the config file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "failed to parse the config file {path}{}",
        .line.map(|line| format!(" at line {line}")).unwrap_or_default()
    )]
    Parse {
        path: PathBuf,
        /// エラーのある行（1 始まり）。
        line: Option<usize>,
        #[source]
        source: Box<toml::de::Error>,
    },
}

impl Config {
    /// `path` のファイルから読む。
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_owned(),
            source,
        })?;
        text.parse().map_err(|mut source: toml::de::Error| {
            // toml のエラーの表示は該当行を抜き出すので、トークンの行がログに残りうる。行の中身は外し、位置は行番号で出す。
            // 閉じない文字列のようにファイルの末尾を指す span は、toml の表示と同じく最後の行に数える。
            let line = source.span().map(|span| {
                text.bytes()
                    .take(span.start.min(text.len().saturating_sub(1)))
                    .filter(|&b| b == b'\n')
                    .count()
                    + 1
            });
            source.set_input(None);
            Error::Parse {
                path: path.to_owned(),
                line,
                source: Box::new(source),
            }
        })
    }
}

impl FromStr for Config {
    type Err = toml::de::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
[misskey]
server = "https://misskey.example.com"
token = "token"
"#;

    /// `text` を一時ファイルに書いて [`Config::load`] で読み、エラーを 1 行にして返す。並行するテストとファイルを分けるため、名前に `name` を入れる。
    fn load_error(name: &str, text: &str) -> String {
        let path =
            std::env::temp_dir().join(format!("tajimi_now-{}-{name}.toml", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let error = Config::load(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        crate::error_chain(&error)
    }

    #[test]
    fn only_the_connection_is_required() {
        let config: Config = MINIMAL.parse().unwrap();
        assert_eq!(
            config.misskey.server.as_str(),
            "https://misskey.example.com/"
        );
        assert_eq!(config.misskey.token, "token");
        assert!(config.amedas.is_none());
        assert!(config.forecast.is_none());
        assert!(config.warning.is_none());
        assert!(config.chance.max_threshold.is_none());
        assert!(config.replace.is_empty());
    }

    #[test]
    fn every_section_is_read() {
        let text = format!(
            r#"{MINIMAL}
[amedas]
point = "52606"
observation_visibility = "specified"

[forecast]
office = "210000"
area = "210010"
visibility = "public"

[warning]
office = "210000"
area = "2120400"

[chance]
max_threshold = 33.4
max_forecast_file = "f1"
max_achieved_file = "f2"
min_threshold = -10

[[replace]]
pattern = '33\.4 ℃'
replacement = ':334:℃'

[[replace]]
pattern = '(?<n>\d+) mm'
replacement = '${{n}}mm'
"#
        );
        let config: Config = text.parse().unwrap();
        let amedas = config.amedas.unwrap();
        assert_eq!(amedas.point, "52606");
        assert_eq!(amedas.observation_visibility, Visibility::Specified);
        assert_eq!(amedas.min_max_visibility, Visibility::Home);
        let forecast = config.forecast.unwrap();
        assert_eq!(
            (forecast.office.as_str(), forecast.area.as_str()),
            ("210000", "210010")
        );
        assert_eq!(forecast.visibility, Visibility::Public);
        let warning = config.warning.unwrap();
        assert_eq!(
            (warning.office.as_str(), warning.area.as_str()),
            ("210000", "2120400")
        );
        assert_eq!(config.chance.max_threshold, Some(33.4));
        assert_eq!(config.chance.max_forecast_file.as_deref(), Some("f1"));
        assert_eq!(config.chance.max_achieved_file.as_deref(), Some("f2"));
        assert_eq!(config.chance.min_threshold, Some(-10.0));
        assert!(config.chance.min_forecast_file.is_none());
        assert_eq!(config.replace.len(), 2);
        assert_eq!(
            config.replace[0]
                .pattern
                .replace("33.4 ℃", &config.replace[0].replacement),
            ":334:℃"
        );
        assert_eq!(
            config.replace[1]
                .pattern
                .replace("12 mm", &config.replace[1].replacement),
            "12mm"
        );
    }

    #[test]
    fn a_missing_token_is_an_error() {
        let error = "[misskey]\nserver = \"https://misskey.example.com\"\n"
            .parse::<Config>()
            .unwrap_err();
        assert!(error.to_string().contains("token"), "{error}");
    }

    #[test]
    fn an_invalid_server_url_is_an_error() {
        let error = "[misskey]\nserver = \"example.com\"\ntoken = \"t\"\n"
            .parse::<Config>()
            .unwrap_err();
        assert!(error.to_string().contains("server"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        let error = format!("{MINIMAL}\n[amedas]\npoint = \"52606\"\nvisibility = \"home\"\n")
            .parse::<Config>()
            .unwrap_err();
        assert!(error.to_string().contains("visibility"), "{error}");
    }

    #[test]
    fn an_unknown_visibility_is_an_error() {
        let error = format!(
            "{MINIMAL}\n[forecast]\noffice = \"1\"\narea = \"2\"\nvisibility = \"direct\"\n"
        )
        .parse::<Config>()
        .unwrap_err();
        assert!(error.to_string().contains("direct"), "{error}");
    }

    #[test]
    fn an_invalid_regex_is_an_error() {
        let error = format!("{MINIMAL}\n[[replace]]\npattern = \"(\"\nreplacement = \"\"\n")
            .parse::<Config>()
            .unwrap_err();
        assert!(error.to_string().contains("regex"), "{error}");
    }

    #[test]
    fn load_reports_a_missing_file() {
        let error = Config::load(Path::new("/nonexistent/config.toml")).unwrap_err();
        assert!(matches!(error, Error::Read { .. }), "{error}");
    }

    #[test]
    fn load_reports_the_line_of_a_parse_error_without_its_text() {
        let text = load_error(
            "array",
            "[misskey]\nserver = \"https://misskey.example.com\"\ntoken = [\"s3cr3t\"]\n",
        );
        assert!(text.contains("at line 3"), "{text}");
        assert!(!text.contains("s3cr3t"), "{text}");
    }

    #[test]
    fn load_counts_an_error_at_the_end_of_the_file_on_the_last_line() {
        let text = load_error(
            "unclosed",
            "[misskey]\nserver = \"https://misskey.example.com\"\ntoken = \"\"\"s3cr3t\n",
        );
        assert!(text.contains("at line 3"), "{text}");
        assert!(!text.contains("s3cr3t"), "{text}");
    }
}
