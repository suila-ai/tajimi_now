//! 前日の枠の観測値 → 前日の気温 → 今日の天気予報 → 今日の枠の観測値の順に、確かめて投稿するジョブ。毎分実行する。
//!
//! 日付が変わった後のタイムラインを、前日の情報 → 当日の情報の順に並べる。前日の気温が載る 24:00 の枠と 00:00 の観測値は
//! 同じレコードで、00:07 ごろに出る。前日の気温を投稿するまで、天気予報と今日の枠の観測値を待たせる。気象庁の遅れや失敗で
//! 投稿できない日は、[`WAIT_UNTIL`] を過ぎたら待たずに進める。前日の枠の観測値を先に確かめるのは、23:50 の枠を投稿しないまま
//! 日付が変わったとき（停止や失敗が続いたとき）に、前日の気温より後ろに並ばないため。
//! 段ごとの失敗は記録して次の段へ進み、1 つの段の失敗で他の投稿を止めない。ピン留めの失敗は投稿済みとして扱い、待たせない。

use chrono::{DateTime, FixedOffset, NaiveTime};
use tracing::error;

use super::{Error, ForecastNote, Job, MinMaxNote, ObservationPoster, PinnedNotePoster};
use crate::jma::Jma;
use crate::misskey::Misskey;

/// 前日の気温を待つ期限（今日の時刻）。24:00 の枠はふだん 00:07 ごろに出る。これを過ぎたら、順序より投稿を優先して待たずに
/// 進める。観測値は直前の枠までしか遡らないので、00:20 を過ぎるまで待つと 00:00 の枠を取りこぼす。
const WAIT_UNTIL: NaiveTime = NaiveTime::from_hms_opt(0, 15, 0).unwrap();

pub struct PostingJob<J, M> {
    /// 前日の気温。`[amedas]` の設定があるとき。
    min_max: Option<PinnedNotePoster<MinMaxNote<J>, M>>,
    /// 今日の天気予報。`[forecast]` の設定があるとき。
    forecast: Option<PinnedNotePoster<ForecastNote<J>, M>>,
    /// 観測値。`[amedas]` の設定があるとき。
    observation: Option<ObservationPoster<J, M>>,
}

impl<J, M> PostingJob<J, M> {
    pub fn new(
        min_max: Option<PinnedNotePoster<MinMaxNote<J>, M>>,
        forecast: Option<PinnedNotePoster<ForecastNote<J>, M>>,
        observation: Option<ObservationPoster<J, M>>,
    ) -> Self {
        Self {
            min_max,
            forecast,
            observation,
        }
    }
}

impl<J: Jma, M: Misskey> PostingJob<J, M> {
    /// `now` の回の分を確かめて投稿する。段の失敗は記録して次の段へ進むので、呼び出し側へは返さない。
    async fn run_at(&self, now: DateTime<FixedOffset>) {
        if let Some(observation) = &self.observation {
            logged("observation", observation.run(now, true).await);
        }
        let yesterday_posted = match &self.min_max {
            Some(min_max) => logged("min_max", min_max.run(now).await).unwrap_or(false),
            None => true,
        };
        let waiting = !yesterday_posted && now.time() < WAIT_UNTIL;
        if !waiting && let Some(forecast) = &self.forecast {
            logged("forecast", forecast.run(now).await);
        }
        if !waiting && let Some(observation) = &self.observation {
            logged("observation", observation.run(now, false).await);
        }
    }
}

impl<J: Jma, M: Misskey> Job for PostingJob<J, M> {
    fn name(&self) -> &'static str {
        "posting"
    }

    async fn run(&self) -> Result<(), Error> {
        self.run_at(crate::time::now()).await;
        Ok(())
    }
}

/// 段の失敗を記録する。成功なら値を返す。
fn logged<T>(step: &'static str, result: Result<T, Error>) -> Option<T> {
    result
        .inspect_err(|error| error!(step, error = %crate::error_chain(error), "step failed"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AmedasConfig, ChanceConfig, ForecastConfig, Visibility};
    use crate::jma::amedas::{Extreme, Reading};
    use crate::jma::fake::FakeJma;
    use crate::jma::{DailyForecast, Observation};
    use crate::misskey::fake::{Fail, FakeMisskey};

    fn at(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }

    /// 24:00 の枠が出た後（00:07）。
    fn after_the_last_slot() -> DateTime<FixedOffset> {
        at("2026-10-23T00:07:00+09:00")
    }

    /// 前日の気温を待っている間（00:03）。
    fn while_waiting() -> DateTime<FixedOffset> {
        at("2026-10-23T00:03:00+09:00")
    }

    /// 待つ期限（00:15）。
    fn at_the_limit() -> DateTime<FixedOffset> {
        at("2026-10-23T00:15:00+09:00")
    }

    /// 昨日の 23:50 と 24:00（今日の 00:00）の枠が出ている気象庁。`extremes` なら、24:00 のレコードに昨日の最高・最低が載る。
    fn jma(extremes: bool) -> FakeJma {
        let extreme = |temperature, time: &str| Extreme {
            temperature,
            time: time.parse().unwrap(),
        };
        let midnight = Observation {
            time: "2026-10-23T00:00:00".parse().unwrap(),
            temperature: Reading::Value(-1.6),
            precipitation_1h: Reading::Value(0.0),
            sunshine_minutes: Reading::Value(0.0),
            wind_speed: Reading::Value(0.8),
            max_temperature: extremes.then(|| extreme(26.8, "14:30:00")),
            min_temperature: extremes.then(|| extreme(-1.6, "00:00:00")),
        };
        FakeJma::new()
            .with_station("多治見")
            .with_observation(Observation {
                time: "2026-10-22T23:50:00".parse().unwrap(),
                max_temperature: None,
                min_temperature: None,
                ..midnight.clone()
            })
            .with_observation(midnight)
            .with_forecast(DailyForecast {
                area_name: "美濃地方".to_owned(),
                weather_code: 100,
                pops: vec![0, 0, 0, 10],
                min_temperature: Some(-2),
                max_temperature: 12,
            })
    }

    /// 23:50 の枠の観測値を 23:57 に投稿してある Misskey。
    fn misskey() -> FakeMisskey {
        FakeMisskey::new(at("2026-10-22T23:57:00+09:00")).with_note("観測値\n(多治見 23:50)")
    }

    fn job<'a>(
        jma: &'a FakeJma,
        misskey: &'a FakeMisskey,
    ) -> PostingJob<&'a FakeJma, &'a FakeMisskey> {
        let amedas = AmedasConfig {
            point: "52606".to_owned(),
            observation_visibility: Visibility::Home,
            min_max_visibility: Visibility::Public,
        };
        let forecast = ForecastConfig {
            office: "210000".to_owned(),
            area: "210010".to_owned(),
            visibility: Visibility::Public,
        };
        let chance = ChanceConfig::default();
        PostingJob::new(
            Some(PinnedNotePoster::new(
                MinMaxNote::new(jma, &amedas, &chance, &[]),
                misskey,
            )),
            Some(PinnedNotePoster::new(
                ForecastNote::new(jma, &forecast, &chance, &[]),
                misskey,
            )),
            Some(ObservationPoster::new(jma, misskey, &amedas, None, &[])),
        )
    }

    /// 投稿されたノートの見出し。観測値はフッタ、前日の気温と天気予報はヘッダ。
    fn headlines(misskey: &FakeMisskey) -> Vec<String> {
        misskey
            .notes()
            .into_iter()
            .map(|note| {
                let header = note.text.lines().next().unwrap_or_default();
                let footer = note.text.lines().last().unwrap_or_default();
                if footer.starts_with('(') {
                    footer
                } else {
                    header
                }
                .to_owned()
            })
            .collect()
    }

    #[tokio::test]
    async fn posts_yesterday_before_today() {
        let (jma, misskey) = (jma(true), misskey());
        job(&jma, &misskey).run_at(after_the_last_slot()).await;
        assert_eq!(
            headlines(&misskey),
            [
                "(多治見 23:50)",
                "昨日(10/22)の気温(多治見)",
                "10/23 天気予報(美濃地方)",
                "(多治見 00:00)"
            ]
        );
        assert_eq!(misskey.pinned_ids(), ["note1", "note2"]);
    }

    #[tokio::test]
    async fn posts_yesterdays_last_slot_before_yesterdays_extremes() {
        // 23:40 の枠を 23:47 に投稿した後に止まり、24:00 の枠が出てから動き出した。
        let jma = jma(true);
        let misskey =
            FakeMisskey::new(at("2026-10-22T23:47:00+09:00")).with_note("観測値\n(多治見 23:40)");
        job(&jma, &misskey).run_at(after_the_last_slot()).await;
        assert_eq!(
            headlines(&misskey),
            [
                "(多治見 23:40)",
                "(多治見 23:50)",
                "昨日(10/22)の気温(多治見)",
                "10/23 天気予報(美濃地方)",
                "(多治見 00:00)"
            ]
        );
    }

    #[tokio::test]
    async fn holds_today_until_yesterday_is_posted() {
        // 00:00 の枠は出ているが、昨日の最高・最低が載っていない。
        let (jma, misskey) = (jma(false), misskey());
        job(&jma, &misskey).run_at(while_waiting()).await;
        assert_eq!(headlines(&misskey), ["(多治見 23:50)"]);
    }

    #[tokio::test]
    async fn stops_waiting_at_the_limit() {
        let (jma, misskey) = (jma(false), misskey());
        job(&jma, &misskey).run_at(at_the_limit()).await;
        assert_eq!(
            headlines(&misskey),
            [
                "(多治見 23:50)",
                "10/23 天気予報(美濃地方)",
                "(多治見 00:00)"
            ]
        );
    }

    #[tokio::test]
    async fn a_pin_failure_does_not_hold_today() {
        let jma = jma(true);
        let misskey = misskey().failing(Fail::Pin);
        job(&jma, &misskey).run_at(after_the_last_slot()).await;
        assert_eq!(
            headlines(&misskey),
            [
                "(多治見 23:50)",
                "昨日(10/22)の気温(多治見)",
                "10/23 天気予報(美濃地方)",
                "(多治見 00:00)"
            ]
        );
        assert!(misskey.pinned_ids().is_empty());
    }
}
