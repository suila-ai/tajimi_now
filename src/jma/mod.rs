//! 気象庁の公開データ（`https://www.jma.go.jp/bosai/`）の取得。

pub mod amedas;

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde::de::DeserializeOwned;
use tokio::sync::OnceCell;

pub use amedas::Observation;

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
}

/// ジョブのテストで使う、決まった結果を返す気象庁。
#[cfg(test)]
pub mod fake {
    use std::collections::BTreeMap;

    use chrono::NaiveDateTime;

    use super::{Error, Jma, Observation};

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
}
