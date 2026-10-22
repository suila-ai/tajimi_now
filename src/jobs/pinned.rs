//! 日に 1 つのノートを投稿してピン留めし、前の日のものと差し替える処理。天気予報と前日の気温で共有する。
//!
//! 投稿済みかは、ピン留めされたノートのヘッダの日付で判定する（状態ファイルは持たない）。その日のノートがピン留めされて
//! いれば、前回外せなかった古いものを外すだけにする。無ければ、データがそろっていれば「投稿 → 古いものを unpin → pin」の順で
//! 差し替える。古いものを先に外すのは、pin の上限に数えさせないため。
//! pin されていないその日のノートが今日の 0 時より後の自分のノートにあれば、投稿せずにそれを pin する。
//! pin に失敗したノートや、エラーになってもサーバでは作られていた投稿（タイムアウト等）を、次の回に二重に投稿しないため。
//! 直近の数件だけを探すと、pin の失敗が続く間に観測値のノートに押し出され、同じ日のノートを投稿し直す。

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeDelta};
use tracing::{error, info, warn};

use super::Error;
use crate::config::{AmedasConfig, ChanceConfig, ForecastConfig, Replace, Visibility};
use crate::jma::{Jma, amedas};
use crate::mfm;
use crate::misskey::{Misskey, NewNote, Note};

/// 今日の自分のノートを古い順に見る数（`users/notes` の上限）。日次のノートは日付が変わった直後に投稿するので最初の数件に入る。
/// 上限まで見るのは、データの遅れで投稿が遅れた日のため（観測値のノートで 16 時間ほどまで）。
const TODAYS_NOTES: u8 = 100;

/// 日に 1 つ投稿するノートの種類。
pub trait DailyNote {
    /// ログに出す名前。
    const NAME: &'static str;
    /// `today`（JST）に投稿するノートが対象にする日。
    fn target_date(today: NaiveDate) -> NaiveDate;
    /// bot のこの種類のノートか。
    fn is_mine(text: &str) -> bool;
    /// `date` を対象にしたノートか。
    fn is_for(text: &str, date: NaiveDate) -> bool;
    /// `date` のノートを組み立てる。データがまだそろっていなければ `None`。
    async fn build(&self, date: NaiveDate) -> Result<Option<NewNote>, crate::jma::Error>;
}

pub struct PinnedNotePoster<D, M> {
    note: D,
    misskey: M,
}

impl<D: DailyNote, M: Misskey> PinnedNotePoster<D, M> {
    pub fn new(note: D, misskey: M) -> Self {
        Self { note, misskey }
    }

    /// 古いノートの pin を外す。失敗しても処理は続ける。
    async fn unpin_quietly(&self, notes: &[&Note]) {
        for note in notes {
            if let Err(error) = self.misskey.unpin(&note.id).await {
                warn!(kind = D::NAME, note_id = %note.id, error = %crate::error_chain(&error), "failed to unpin an old note");
            }
        }
    }

    /// 対象の日のノートを投稿してピン留めする。そのノートがあれば（今投稿したものも含む）true、データがまだそろっていなければ false。
    /// ピン留めの失敗は記録だけして true を返し、次の回にやり直す。投稿の順序には、ノートがあることだけが効くため。
    pub async fn run(&self, now: DateTime<FixedOffset>) -> Result<bool, Error> {
        let date = D::target_date(now.date_naive());
        let pinned = self.misskey.my_pinned_notes().await?;
        let (current, old): (Vec<&Note>, Vec<&Note>) = pinned
            .iter()
            .filter(|note| D::is_mine(&note.text))
            .partition(|note| D::is_for(&note.text, date));
        if !current.is_empty() {
            // 前回の差し替えで外せなかった古いノートが残っていれば、ここで外す。
            self.unpin_quietly(&old).await;
            return Ok(true);
        }

        // どちらの種類も今日（JST）のうちに投稿する。
        let midnight = now
            .with_time(NaiveTime::MIN)
            .single()
            .expect("a fixed offset has exactly one midnight");
        let todays = self
            .misskey
            .my_notes_since(midnight.to_utc(), TODAYS_NOTES)
            .await?;
        let note = match todays.into_iter().find(|note| D::is_for(&note.text, date)) {
            Some(note) => note,
            None => {
                let Some(new_note) = self.note.build(date).await? else {
                    return Ok(false);
                };
                let note = self.misskey.create_note(&new_note).await?;
                info!(kind = D::NAME, note_id = %note.id, %date, files = new_note.file_ids.len(), "posted");
                note
            }
        };
        self.unpin_quietly(&old).await;
        match self.misskey.pin(&note.id).await {
            Ok(()) => info!(kind = D::NAME, note_id = %note.id, %date, "pinned"),
            Err(error) => {
                error!(kind = D::NAME, note_id = %note.id, error = %crate::error_chain(&error), "failed to pin; retrying on the next run");
            }
        }
        Ok(true)
    }
}

/// 今日の天気予報。
pub struct ForecastNote<J> {
    jma: J,
    office: String,
    area: String,
    visibility: Visibility,
    chance: ChanceConfig,
    replacements: Vec<Replace>,
}

impl<J: Jma> ForecastNote<J> {
    pub fn new(
        jma: J,
        config: &ForecastConfig,
        chance: &ChanceConfig,
        replacements: &[Replace],
    ) -> Self {
        Self {
            jma,
            office: config.office.clone(),
            area: config.area.clone(),
            visibility: config.visibility,
            chance: chance.clone(),
            replacements: replacements.to_vec(),
        }
    }
}

impl<J: Jma> DailyNote for ForecastNote<J> {
    const NAME: &'static str = "forecast";

    fn target_date(today: NaiveDate) -> NaiveDate {
        today
    }

    fn is_mine(text: &str) -> bool {
        mfm::forecast::is_mine(text)
    }

    fn is_for(text: &str, date: NaiveDate) -> bool {
        mfm::forecast::is_for(text, date)
    }

    async fn build(&self, date: NaiveDate) -> Result<Option<NewNote>, crate::jma::Error> {
        let Some(forecast) = self
            .jma
            .daily_forecast(&self.office, &self.area, date)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(NewNote {
            text: mfm::forecast::render(date, &forecast, &self.replacements),
            visibility: self.visibility,
            file_ids: self.chance.forecast_files(
                forecast.min_temperature.map(f64::from),
                f64::from(forecast.max_temperature),
            ),
        }))
    }
}

/// 前日の最高・最低気温。
pub struct MinMaxNote<J> {
    jma: J,
    point: String,
    visibility: Visibility,
    chance: ChanceConfig,
    replacements: Vec<Replace>,
}

impl<J: Jma> MinMaxNote<J> {
    pub fn new(
        jma: J,
        config: &AmedasConfig,
        chance: &ChanceConfig,
        replacements: &[Replace],
    ) -> Self {
        Self {
            jma,
            point: config.point.clone(),
            visibility: config.min_max_visibility,
            chance: chance.clone(),
            replacements: replacements.to_vec(),
        }
    }
}

impl<J: Jma> DailyNote for MinMaxNote<J> {
    const NAME: &'static str = "min_max";

    fn target_date(today: NaiveDate) -> NaiveDate {
        today - TimeDelta::days(1)
    }

    fn is_mine(text: &str) -> bool {
        mfm::min_max::is_mine(text)
    }

    fn is_for(text: &str, date: NaiveDate) -> bool {
        mfm::min_max::is_for(text, date)
    }

    /// その日の最後の枠（24:00）が出ていれば、そのレコードに載る日最高・最低で組み立てる。気温の無い地点では載らないので投稿しない。
    async fn build(&self, date: NaiveDate) -> Result<Option<NewNote>, crate::jma::Error> {
        let Some(observation) = self
            .jma
            .observation_at(&self.point, amedas::day_end(date))
            .await?
        else {
            return Ok(None);
        };
        let (Some(max), Some(min)) = (observation.max_temperature, observation.min_temperature)
        else {
            return Ok(None);
        };
        let station = self.jma.station_name(&self.point).await?;
        Ok(Some(NewNote {
            text: mfm::min_max::render(date, &station, &max, &min, &self.replacements),
            visibility: self.visibility,
            file_ids: self.chance.achieved_files(min.temperature, max.temperature),
        }))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset};

    use super::*;
    use crate::jma::amedas::{Extreme, Reading};
    use crate::jma::fake::FakeJma;
    use crate::jma::{DailyForecast, Observation};
    use crate::misskey::fake::{Fail, FakeMisskey};

    /// テストの「今」。2026-10-23 05:12 JST。
    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-23T05:12:00+09:00").unwrap()
    }

    fn forecast() -> DailyForecast {
        DailyForecast {
            area_name: "美濃地方".to_owned(),
            weather_code: 100,
            pops: vec![0, 10, 20],
            min_temperature: None,
            max_temperature: 22,
        }
    }

    fn chance() -> ChanceConfig {
        ChanceConfig {
            max_threshold: Some(20.0),
            max_forecast_file: Some("forecast-hot".to_owned()),
            max_achieved_file: Some("achieved-hot".to_owned()),
            min_threshold: Some(0.0),
            min_forecast_file: Some("forecast-cold".to_owned()),
            min_achieved_file: Some("achieved-cold".to_owned()),
        }
    }

    fn forecast_job(
        jma: FakeJma,
        misskey: &FakeMisskey,
    ) -> PinnedNotePoster<ForecastNote<FakeJma>, &FakeMisskey> {
        let config = ForecastConfig {
            office: "210000".to_owned(),
            area: "210010".to_owned(),
            visibility: Visibility::Public,
        };
        PinnedNotePoster::new(ForecastNote::new(jma, &config, &chance(), &[]), misskey)
    }

    const TODAY: &str = "10/23 天気予報(美濃地方)\n$[x2 ☀] 晴れ\n降水確率: 0→10→20 %\n気温: ↑22 ℃";
    const YESTERDAY: &str = "10/22 天気予報(美濃地方)\n$[x2 ☀] 晴れ\n降水確率: 0 %\n気温: ↑20 ℃";

    #[tokio::test]
    async fn posts_pins_and_attaches_when_nothing_is_pinned() {
        let misskey = FakeMisskey::new(now());
        assert!(
            forecast_job(FakeJma::new().with_forecast(forecast()), &misskey)
                .run(now())
                .await
                .unwrap()
        );

        let notes = misskey.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, TODAY);
        assert_eq!(misskey.pinned_ids(), [notes[0].id.clone()]);
        // 最高気温 22 が閾値 20 を超えたので添付する。最低気温は無いので添付しない。
        assert_eq!(misskey.file_ids_of(&notes[0].id), ["forecast-hot"]);
    }

    #[tokio::test]
    async fn replaces_the_old_note_and_leaves_unrelated_pins_alone() {
        // 上限いっぱいでも、古いものを先に外すので差し替えられる。
        let misskey = FakeMisskey::new(now())
            .with_pinned_note(YESTERDAY)
            .with_pinned_note("関係ないノート")
            .with_pin_limit(2);
        assert!(
            forecast_job(FakeJma::new().with_forecast(forecast()), &misskey)
                .run(now())
                .await
                .unwrap()
        );

        assert_eq!(misskey.notes().len(), 3);
        assert_eq!(misskey.pinned_ids(), ["note1", "note2"]);
    }

    #[tokio::test]
    async fn does_nothing_when_today_is_already_pinned() {
        let misskey = FakeMisskey::new(now())
            .with_pinned_note(YESTERDAY)
            .with_pinned_note(TODAY);
        assert!(
            forecast_job(FakeJma::new().with_forecast(forecast()), &misskey)
                .run(now())
                .await
                .unwrap()
        );

        assert_eq!(misskey.notes().len(), 2);
        // 前回外せなかった古いものは外す。
        assert_eq!(misskey.pinned_ids(), ["note1"]);
    }

    #[tokio::test]
    async fn does_nothing_until_the_data_is_available() {
        let misskey = FakeMisskey::new(now()).with_pinned_note(YESTERDAY);
        assert!(
            !forecast_job(FakeJma::new(), &misskey)
                .run(now())
                .await
                .unwrap()
        );
        assert_eq!(misskey.notes().len(), 1);
        assert_eq!(misskey.pinned_ids(), ["note0"]);
    }

    #[tokio::test]
    async fn keeps_the_note_and_pins_it_on_the_next_run_when_pinning_fails() {
        let misskey = FakeMisskey::new(now())
            .with_pinned_note(YESTERDAY)
            .failing(Fail::Pin);
        let job = forecast_job(FakeJma::new().with_forecast(forecast()), &misskey);
        // 投稿は残す。古いものは pin の前に外している。ノートはあるので true。
        assert!(job.run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 2);
        assert!(misskey.pinned_ids().is_empty());

        misskey.recover();
        assert!(job.run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 2);
        assert_eq!(misskey.pinned_ids(), ["note1"]);
    }

    #[tokio::test]
    async fn pins_a_note_saved_despite_an_error_without_posting_again() {
        let misskey = FakeMisskey::new(now())
            .with_pinned_note(YESTERDAY)
            .failing(Fail::CreateTimeout);
        let job = forecast_job(FakeJma::new().with_forecast(forecast()), &misskey);
        assert!(job.run(now()).await.is_err());
        assert_eq!(misskey.notes().len(), 2);

        misskey.recover();
        assert!(job.run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 2);
        assert_eq!(misskey.pinned_ids(), ["note1"]);
    }

    #[tokio::test]
    async fn finds_todays_note_behind_later_notes_while_pinning_keeps_failing() {
        let misskey = FakeMisskey::new(now()).with_note(TODAY).failing(Fail::Pin);
        let misskey = (0..20).fold(misskey, |misskey, _| misskey.with_note("観測値"));
        let job = forecast_job(FakeJma::new().with_forecast(forecast()), &misskey);
        assert!(job.run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 21);

        // 直近 10 件には今日のノートが入らないが、今日の 0 時以降として見つけて pin する。
        misskey.recover();
        assert!(job.run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 21);
        assert_eq!(misskey.pinned_ids(), ["note0"]);
    }

    #[tokio::test]
    async fn fails_without_posting_when_misskey_cannot_be_read() {
        let misskey = FakeMisskey::new(now()).failing(Fail::Read);
        let error = forecast_job(FakeJma::new().with_forecast(forecast()), &misskey)
            .run(now())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Misskey(_)), "{error}");
        assert!(misskey.notes().is_empty());
    }

    fn observation(time: &str) -> Observation {
        Observation {
            time: time.parse().unwrap(),
            temperature: Reading::Absent,
            precipitation_1h: Reading::Absent,
            sunshine_minutes: Reading::Absent,
            wind_speed: Reading::Absent,
            max_temperature: None,
            min_temperature: None,
        }
    }

    fn extreme(temperature: f64, time: &str) -> Option<Extreme> {
        Some(Extreme {
            temperature,
            time: time.parse().unwrap(),
        })
    }

    fn min_max_job(
        jma: FakeJma,
        misskey: &FakeMisskey,
    ) -> PinnedNotePoster<MinMaxNote<FakeJma>, &FakeMisskey> {
        let config = AmedasConfig {
            point: "52606".to_owned(),
            observation_visibility: Visibility::Home,
            min_max_visibility: Visibility::Public,
        };
        PinnedNotePoster::new(MinMaxNote::new(jma, &config, &chance(), &[]), misskey)
    }

    /// 昨日の 24:00 の枠。最低は 24:00 に出た（起時は 00:00 で持つ）。
    fn yesterday() -> FakeJma {
        FakeJma::new()
            .with_station("多治見")
            .with_observation(Observation {
                max_temperature: extreme(26.8, "14:30:00"),
                min_temperature: extreme(-1.6, "00:00:00"),
                ..observation("2026-10-23T00:00:00")
            })
    }

    #[tokio::test]
    async fn posts_yesterdays_extremes_with_attachments() {
        let misskey = FakeMisskey::new(now());
        assert!(min_max_job(yesterday(), &misskey).run(now()).await.unwrap());

        let notes = misskey.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(
            notes[0].text,
            "昨日(10/22)の気温(多治見)\n最高: 26.8 ℃ (14:30)\n最低: -1.6 ℃ (24:00)"
        );
        assert_eq!(misskey.pinned_ids(), ["note0"]);
        assert_eq!(
            misskey.file_ids_of("note0"),
            ["achieved-hot", "achieved-cold"]
        );
    }

    #[tokio::test]
    async fn waits_for_the_last_slot_of_the_day() {
        let misskey = FakeMisskey::new(now());
        let jma = FakeJma::new()
            .with_station("多治見")
            .with_observation(observation("2026-10-22T23:50:00"));
        assert!(!min_max_job(jma, &misskey).run(now()).await.unwrap());
        assert!(misskey.notes().is_empty());

        // 最後の枠はあるが、最高・最低が載っていない。
        let jma = FakeJma::new()
            .with_station("多治見")
            .with_observation(observation("2026-10-23T00:00:00"));
        assert!(!min_max_job(jma, &misskey).run(now()).await.unwrap());
        assert!(misskey.notes().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_station_is_an_error_even_before_the_last_slot() {
        let misskey = FakeMisskey::new(now());
        let error = min_max_job(FakeJma::new(), &misskey)
            .run(now())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Jma(_)), "{error}");
        assert!(misskey.notes().is_empty());
    }

    #[tokio::test]
    async fn does_nothing_when_yesterday_is_already_pinned() {
        let misskey = FakeMisskey::new(now())
            .with_pinned_note("昨日(10/22)の気温(多治見)\n最高: 1 ℃ (00:00)\n最低: 1 ℃ (00:00)");
        assert!(min_max_job(yesterday(), &misskey).run(now()).await.unwrap());
        assert_eq!(misskey.notes().len(), 1);
    }
}
