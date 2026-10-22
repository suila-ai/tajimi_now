//! 気象庁の公開データ（`https://www.jma.go.jp/bosai/`）の取得。

pub mod amedas;
pub mod forecast;
pub mod warning;
pub mod weather;

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime};
use serde::de::DeserializeOwned;
use tokio::sync::OnceCell;

pub use amedas::Observation;
pub use forecast::DailyForecast;
pub use warning::Kind;

const ORIGIN: &str = "https://www.jma.go.jp";

/// ジョブが気象庁に対して行う取得。
pub trait Jma {
    /// アメダス地点の名前（`多治見`）。
    async fn station_name(&self, point: &str) -> Result<String, Error>;
    /// 10 分枠 `slot`（[`amedas::slot`]）の観測値。その枠の値がまだ無ければ `None`。
    /// 1 日の最後の枠（[`amedas::day_end`]）の観測値には、その日の最高・最低気温が載る。
    /// `point` が地点表に無ければ [`Error::UnknownStation`]。
    async fn observation_at(
        &self,
        point: &str,
        slot: NaiveDateTime,
    ) -> Result<Option<Observation>, Error>;
    /// `office` が発表する `area`（一次細分区域）の `date` の予報。天気・降水確率・最高気温がまだ無ければ `None`。
    /// `area` が `office` の発表に無ければ [`Error::UnknownArea`]。
    async fn daily_forecast(
        &self,
        office: &str,
        area: &str,
        date: NaiveDate,
    ) -> Result<Option<DailyForecast>, Error>;
    /// `office` が `area`（市区町村）に発表中の警報・注意報。[`warning::TABLE`] の順。
    /// `area` が `office` の発表に無ければ [`Error::UnknownArea`]。
    async fn active_warnings(&self, office: &str, area: &str) -> Result<Vec<Kind>, Error>;
}

/// 参照越しにも使えるようにする。1 つのクライアントを複数のジョブで共有するため。
impl<T: Jma> Jma for &T {
    async fn station_name(&self, point: &str) -> Result<String, Error> {
        (**self).station_name(point).await
    }

    async fn observation_at(
        &self,
        point: &str,
        slot: NaiveDateTime,
    ) -> Result<Option<Observation>, Error> {
        (**self).observation_at(point, slot).await
    }

    async fn daily_forecast(
        &self,
        office: &str,
        area: &str,
        date: NaiveDate,
    ) -> Result<Option<DailyForecast>, Error> {
        (**self).daily_forecast(office, area, date).await
    }

    async fn active_warnings(&self, office: &str, area: &str) -> Result<Vec<Kind>, Error> {
        (**self).active_warnings(office, area).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP request to {url} failed")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to parse the response from {url}")]
    Parse {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{url} was not found")]
    NotFound { url: String },
    #[error("unknown AMeDAS station {point}")]
    UnknownStation { point: String },
    #[error("area {area} is not in the reports of office {office}")]
    UnknownArea { office: String, area: String },
}

/// 気象庁のサイトを叩く実装。
#[derive(Debug)]
pub struct JmaClient {
    client: reqwest::Client,
    /// 地点表。最初に要るときに取り、以後は使い回す。
    stations: OnceCell<HashMap<String, amedas::Station>>,
    /// `bosai` のベース URL。テストではモックサーバに向ける。
    origin: String,
}

impl JmaClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_origin(client, ORIGIN)
    }

    fn with_origin(client: reqwest::Client, origin: &str) -> Self {
        Self {
            client,
            stations: OnceCell::new(),
            origin: origin.to_owned(),
        }
    }

    /// `path`（`/bosai/...`）の JSON を取って `T` に読む。無い（404）なら `None`。
    async fn fetch<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, Error> {
        let url = format!("{}{path}", self.origin);
        let http = |source| Error::Http {
            url: url.clone(),
            source,
        };
        let response = self.client.get(&url).send().await.map_err(http)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = response
            .error_for_status()
            .map_err(http)?
            .bytes()
            .await
            .map_err(http)?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|source| Error::Parse { url, source })
    }

    /// `path` の JSON を取る。無い（404）ならエラー。
    async fn fetch_required<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.fetch(path).await?.ok_or_else(|| Error::NotFound {
            url: format!("{}{path}", self.origin),
        })
    }

    /// 地点表の `point`。
    async fn station(&self, point: &str) -> Result<&amedas::Station, Error> {
        let stations = self
            .stations
            .get_or_try_init(|| self.fetch_required(amedas::STATIONS_PATH))
            .await?;
        stations.get(point).ok_or_else(|| Error::UnknownStation {
            point: point.to_owned(),
        })
    }
}

impl Jma for JmaClient {
    async fn station_name(&self, point: &str) -> Result<String, Error> {
        Ok(self.station(point).await?.name.clone())
    }

    async fn observation_at(
        &self,
        point: &str,
        slot: NaiveDateTime,
    ) -> Result<Option<Observation>, Error> {
        // 地点コードの誤りは、枠のファイルが無い（まだ出ていない）のと区別できないので、先に地点表で確かめる。
        self.station(point).await?;
        let Some(block) = self.fetch(&amedas::block_path(point, slot)).await? else {
            return Ok(None);
        };
        Ok(amedas::observation(&block, slot))
    }

    async fn daily_forecast(
        &self,
        office: &str,
        area: &str,
        date: NaiveDate,
    ) -> Result<Option<DailyForecast>, Error> {
        let reports: forecast::Reports = self.fetch_required(&forecast::path(office)).await?;
        // 区域が無いのも予報がまだ無いのも `daily` では `None` になり、コードを誤ると黙って投稿しなくなるので、分けてエラーにする。
        if !forecast::covers(&reports, area) {
            return Err(Error::UnknownArea {
                office: office.to_owned(),
                area: area.to_owned(),
            });
        }
        // 対応表は使い回さず、毎回取る。使い回すと、気象庁が区域の気温の地点を差し替えたときに、再起動するまで気温が引けず、
        // 予報が黙って止まる。引くのはその日の予報を投稿するまでの間（ふだんは 0 時台の 1 回）だけなので、毎回取っても負担は無い。
        let table: forecast::AreaTable = self.fetch_required(forecast::AREAS_PATH).await?;
        Ok(forecast::daily(&reports, &table, area, date))
    }

    async fn active_warnings(&self, office: &str, area: &str) -> Result<Vec<Kind>, Error> {
        let reports: warning::Reports = self.fetch_required(&warning::path(office)).await?;
        // 市区町村の誤りは、発表が無いのと同じ空の結果になるので、エラーにして気付けるようにする。
        if !warning::covers(&reports, area) {
            return Err(Error::UnknownArea {
                office: office.to_owned(),
                area: area.to_owned(),
            });
        }
        Ok(warning::active(&reports, area))
    }
}

/// ジョブのテストで使う、決まった結果を返す気象庁。
#[cfg(test)]
pub mod fake {
    use std::collections::BTreeMap;

    use chrono::{NaiveDate, NaiveDateTime};

    use super::{DailyForecast, Error, Jma, Kind, Observation};

    /// 取得に失敗したことを表すエラー。
    pub fn fetch_error() -> Error {
        Error::UnknownStation {
            point: "fake".to_owned(),
        }
    }

    #[derive(Debug, Default)]
    pub struct FakeJma {
        /// 地点名。無ければ地点表の取得に失敗し、観測値も取れない。
        station: Option<String>,
        /// 10 分枠ごとの観測値。
        observations: BTreeMap<NaiveDateTime, Observation>,
        /// どの日を聞かれても返す予報。無ければ「まだ無い」。
        forecast: Option<DailyForecast>,
        /// 発表中の警報・注意報。`None` なら取得に失敗する。
        warnings: Option<Vec<Kind>>,
    }

    impl FakeJma {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_station(mut self, name: &str) -> Self {
            self.station = Some(name.to_owned());
            self
        }

        pub fn with_observation(mut self, observation: Observation) -> Self {
            self.observations.insert(observation.time, observation);
            self
        }

        pub fn with_forecast(mut self, forecast: DailyForecast) -> Self {
            self.forecast = Some(forecast);
            self
        }

        pub fn with_warnings(mut self, kinds: Vec<Kind>) -> Self {
            self.warnings = Some(kinds);
            self
        }
    }

    impl Jma for FakeJma {
        async fn station_name(&self, _point: &str) -> Result<String, Error> {
            self.station.clone().ok_or_else(fetch_error)
        }

        async fn observation_at(
            &self,
            _point: &str,
            slot: NaiveDateTime,
        ) -> Result<Option<Observation>, Error> {
            self.station.as_ref().ok_or_else(fetch_error)?;
            Ok(self.observations.get(&slot).cloned())
        }

        async fn daily_forecast(
            &self,
            _office: &str,
            _area: &str,
            _date: NaiveDate,
        ) -> Result<Option<DailyForecast>, Error> {
            Ok(self.forecast.clone())
        }

        async fn active_warnings(&self, _office: &str, _area: &str) -> Result<Vec<Kind>, Error> {
            self.warnings.clone().ok_or_else(fetch_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::time::JST;

    const STATIONS: &str = include_str!("../../tests/fixtures/jma/amedastable.json");
    const BLOCK_21: &str = include_str!("../../tests/fixtures/jma/amedas_52606_20261022_21.json");
    const AREAS: &str = include_str!("../../tests/fixtures/jma/forecast_area.json");
    const FORECAST: &str = include_str!("../../tests/fixtures/jma/forecast_210000_1700.json");
    const WARNING: &str = include_str!("../../tests/fixtures/jma/warning_474000.json");

    async fn mount(server: &MockServer, url_path: &str, body: &str, expect: u64) {
        Mock::given(method("GET"))
            .and(path(url_path))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(expect)
            .mount(server)
            .await;
    }

    fn naive(value: &str) -> NaiveDateTime {
        value.parse().unwrap()
    }

    #[tokio::test]
    async fn the_station_table_is_fetched_once() {
        let server = MockServer::start().await;
        mount(&server, "/bosai/amedas/const/amedastable.json", STATIONS, 1).await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());

        assert_eq!(jma.station_name("52606").await.unwrap(), "多治見");
        assert_eq!(jma.station_name("52586").await.unwrap(), "岐阜");
        let error = jma.station_name("00000").await.unwrap_err();
        assert!(
            matches!(error, Error::UnknownStation { ref point } if point == "00000"),
            "{error}"
        );
        // 観測値も、枠のファイルを取る前に地点を確かめる。
        let error = jma
            .observation_at("00000", naive("2026-10-22T21:00:00"))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::UnknownStation { .. }), "{error}");
    }

    #[tokio::test]
    async fn the_area_table_is_fetched_on_each_call() {
        let server = MockServer::start().await;
        mount(
            &server,
            "/bosai/forecast/data/forecast/210000.json",
            FORECAST,
            2,
        )
        .await;
        mount(
            &server,
            "/bosai/forecast/const/forecast_area.json",
            AREAS,
            2,
        )
        .await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        for _ in 0..2 {
            assert!(
                jma.daily_forecast("210000", "210010", date)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn observation_at_reads_the_slot_from_its_block() {
        let server = MockServer::start().await;
        mount(&server, "/bosai/amedas/const/amedastable.json", STATIONS, 1).await;
        mount(
            &server,
            "/bosai/amedas/data/point/52606/20261022_21.json",
            BLOCK_21,
            2,
        )
        .await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());

        let observation = jma
            .observation_at("52606", naive("2026-10-22T21:10:00"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observation.time, naive("2026-10-22T21:10:00"));
        assert_eq!(observation.temperature, amedas::Reading::Value(13.8));

        // ファイルはあるが、その枠の値がまだ無い。
        assert!(
            jma.observation_at("52606", naive("2026-10-22T22:20:00"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_missing_block_means_no_observation_yet() {
        let server = MockServer::start().await;
        mount(&server, "/bosai/amedas/const/amedastable.json", STATIONS, 1).await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        assert!(
            jma.observation_at("52606", naive("2026-10-22T21:00:00"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn server_errors_are_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let error = jma
            .observation_at("52606", naive("2026-10-22T21:00:00"))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http { .. }), "{error}");
        let error = jma.station_name("52606").await.unwrap_err();
        assert!(matches!(error, Error::Http { .. }), "{error}");
    }

    #[tokio::test]
    async fn a_broken_block_is_a_parse_error() {
        let server = MockServer::start().await;
        mount(&server, "/bosai/amedas/const/amedastable.json", STATIONS, 1).await;
        mount(
            &server,
            "/bosai/amedas/data/point/52606/20261022_21.json",
            "{",
            1,
        )
        .await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());

        let error = jma
            .observation_at("52606", naive("2026-10-22T21:00:00"))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Parse { .. }), "{error}");
    }

    #[tokio::test]
    async fn an_area_outside_the_office_is_an_error() {
        let server = MockServer::start().await;
        mount(
            &server,
            "/bosai/forecast/data/forecast/210000.json",
            FORECAST,
            1,
        )
        .await;
        // 区域の対応表は mount しない。区域の確認が表の取得より前にあることも確かめる。
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let error = jma
            .daily_forecast("210000", "130010", date)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::UnknownArea { .. }), "{error}");

        // 別の気象台の市区町村。
        mount(&server, "/bosai/warning/data/r8/474000.json", WARNING, 1).await;
        let error = jma.active_warnings("474000", "2120400").await.unwrap_err();
        assert!(matches!(error, Error::UnknownArea { .. }), "{error}");
    }

    #[tokio::test]
    async fn daily_forecast_is_read_from_the_office_file() {
        let server = MockServer::start().await;
        mount(
            &server,
            "/bosai/forecast/data/forecast/210000.json",
            FORECAST,
            1,
        )
        .await;
        mount(
            &server,
            "/bosai/forecast/const/forecast_area.json",
            AREAS,
            1,
        )
        .await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let forecast = jma
            .daily_forecast("210000", "210010", date)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(forecast.area_name, "美濃地方");
        assert_eq!(forecast.max_temperature, 22);
    }

    #[tokio::test]
    async fn active_warnings_are_read_from_the_office_file() {
        let server = MockServer::start().await;
        mount(&server, "/bosai/warning/data/r8/474000.json", WARNING, 1).await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let kinds = jma.active_warnings("474000", "4738100").await.unwrap();
        assert_eq!(kinds.len(), 6);
        assert_eq!(kinds[0].code, "10");
    }

    #[tokio::test]
    async fn a_missing_office_file_is_an_error() {
        let server = MockServer::start().await;
        let jma = JmaClient::with_origin(reqwest::Client::new(), &server.uri());
        let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let error = jma
            .daily_forecast("210000", "210010", date)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::NotFound { .. }), "{error}");
        let error = jma.active_warnings("210000", "2120400").await.unwrap_err();
        assert!(matches!(error, Error::NotFound { .. }), "{error}");
    }

    /// 実際の気象庁のサイトに対する取得。手で実行する。
    #[tokio::test]
    #[ignore = "needs network access to www.jma.go.jp"]
    async fn live_jma_amedas() {
        let jma = JmaClient::new(reqwest::Client::new());
        assert_eq!(jma.station_name("52606").await.unwrap(), "多治見");
        let now = chrono::Utc::now().with_timezone(&JST);
        // 直前の枠は出ていないことがあるので、20 分前の枠を見る。
        let slot = amedas::slot(now - TimeDelta::minutes(20));
        let observation = jma
            .observation_at("52606", slot)
            .await
            .unwrap()
            .expect("an observation 20 minutes ago");
        assert_eq!(observation.time, slot);
        // 昨日の 24:00 の枠には、昨日の最高・最低が載る。
        let yesterday = (now - TimeDelta::days(1)).date_naive();
        let observation = jma
            .observation_at("52606", amedas::day_end(yesterday))
            .await
            .unwrap()
            .expect("the 24:00 slot of yesterday");
        assert!(observation.max_temperature.is_some(), "{observation:?}");
        assert!(observation.min_temperature.is_some(), "{observation:?}");
    }

    /// 実際の気象庁のサイトに対する取得。手で実行する。
    #[tokio::test]
    #[ignore = "needs network access to www.jma.go.jp"]
    async fn live_jma_forecast() {
        let jma = JmaClient::new(reqwest::Client::new());
        // 最新の発表の日の翌日なら、天気と最高気温がある。0〜5 時の最新は前日 17 時の発表なので、
        // 6 時間前の日の翌日（18 時間後の日）を見る。
        let date = (chrono::Utc::now().with_timezone(&JST) + TimeDelta::hours(18)).date_naive();
        let forecast = jma
            .daily_forecast("210000", "210010", date)
            .await
            .unwrap()
            .expect("the forecast for the day after the latest report");
        assert_eq!(forecast.area_name, "美濃地方");
        assert!(weather::Weather::from_code(forecast.weather_code).is_some());
    }

    /// 実際の気象庁のサイトに対する取得。手で実行する。発表中の種別は日によって違うので、読めることだけを見る。
    #[tokio::test]
    #[ignore = "needs network access to www.jma.go.jp"]
    async fn live_jma_warnings() {
        let jma = JmaClient::new(reqwest::Client::new());
        let kinds = jma.active_warnings("210000", "2120400").await.unwrap();
        assert!(kinds.iter().all(|kind| warning::TABLE.contains(kind)));
    }
}
