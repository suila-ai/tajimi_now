//! アメダスの観測値を 10 分枠ごとに投稿する処理。
//!
//! 毎分実行し、直前と今の 10 分枠のうち、値が出ていてまだ投稿していない古い方を投稿する。直前の枠も見るのは、
//! 値の公開が次の枠に入ってからになっても取りこぼさないため。
//! 最後に投稿した枠はメモリに持つ。起動時と、投稿の応答を待つ間や失敗した後は分からないものとし、自分の直近のノートの
//! フッタから読み戻す。再起動や、応答の前にエラーや打ち切り（scheduler の `RUN_TIMEOUT`）になってもサーバでは作られていた
//! 投稿で、同じ枠を二重に投稿しないため。

use std::sync::Mutex;

use chrono::{DateTime, FixedOffset, NaiveDateTime, NaiveTime};
use tracing::{info, warn};

use super::Error;
use crate::config::{AmedasConfig, Replace, Visibility, WarningConfig};
use crate::jma::{Jma, Kind, Observation, amedas};
use crate::mfm;
use crate::misskey::{Misskey, NewNote};

/// 自分の直近のノートから、最後に投稿した枠を探すときに見る数。観測値のノートは 10 分に 1 つ、日次のノートは日に 2 つなので、
/// 直前に投稿したものは直近の数件に必ず入る。
const RECENT_NOTES: u8 = 10;

pub struct ObservationPoster<J, M> {
    jma: J,
    misskey: M,
    point: String,
    visibility: Visibility,
    warning: Option<WarningConfig>,
    replacements: Vec<Replace>,
    /// 最後に投稿した枠。起動直後と、投稿の応答を待つ間・失敗した後は分からない。
    last_posted: Mutex<LastPosted>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastPosted {
    Unknown,
    Known(Option<NaiveDateTime>),
}

impl<J: Jma, M: Misskey> ObservationPoster<J, M> {
    pub fn new(
        jma: J,
        misskey: M,
        amedas: &AmedasConfig,
        warning: Option<&WarningConfig>,
        replacements: &[Replace],
    ) -> Self {
        Self {
            jma,
            misskey,
            point: amedas.point.clone(),
            visibility: amedas.observation_visibility,
            warning: warning.cloned(),
            replacements: replacements.to_vec(),
            last_posted: Mutex::new(LastPosted::Unknown),
        }
    }

    /// 最後に投稿した枠。分からなければ、自分の直近のノートから読み戻す。
    async fn last_posted(&self) -> Result<Option<NaiveDateTime>, Error> {
        if let LastPosted::Known(slot) = *self.last_posted.lock().unwrap() {
            return Ok(slot);
        }
        let notes = self.misskey.my_recent_notes(RECENT_NOTES).await?;
        let station = self.jma.station_name(&self.point).await?;
        let slot = notes
            .iter()
            .find_map(|note| mfm::observation::posted_slot(&note.text, &station, note.created_at));
        info!(slot = ?slot, "restored the last posted slot from recent notes");
        *self.last_posted.lock().unwrap() = LastPosted::Known(slot);
        Ok(slot)
    }

    /// 発表中の警報・注意報。設定が無ければ空。取得に失敗しても観測値は投稿するので、警告して空にする。
    async fn warnings(&self) -> Vec<Kind> {
        let Some(warning) = &self.warning else {
            return Vec::new();
        };
        match self
            .jma
            .active_warnings(&warning.office, &warning.area)
            .await
        {
            Ok(kinds) => kinds,
            Err(error) => {
                warn!(error = %crate::error_chain(&error), "posting without warnings");
                Vec::new()
            }
        }
    }

    /// 直前と今の 10 分枠のうち、値が出ていてまだ投稿していない古い方を投稿する。`hold_today` なら今日の枠（00:00 以降）は
    /// 投稿しない。前日の気温より先に今日の観測値を出さないため（[`PostingJob`](super::PostingJob)）。
    /// 対象の枠が無ければ何も取得しない（`PostingJob` が毎分、前日の枠だけを先に確かめるため）。
    pub async fn run(&self, now: DateTime<FixedOffset>, hold_today: bool) -> Result<(), Error> {
        let current = amedas::slot(now);
        let today = current.date().and_time(NaiveTime::MIN);
        for slot in [current - amedas::SLOT, current] {
            if hold_today && slot >= today {
                break;
            }
            if self.last_posted().await?.is_some_and(|last| last >= slot) {
                continue;
            }
            if let Some(observation) = self.jma.observation_at(&self.point, slot).await? {
                return self.create(&observation).await;
            }
        }
        Ok(())
    }

    /// 観測値を投稿し、投稿した枠を覚える。
    async fn create(&self, observation: &Observation) -> Result<(), Error> {
        let station = self.jma.station_name(&self.point).await?;
        let warnings = self.warnings().await;
        let text = mfm::observation::render(&station, observation, &warnings, &self.replacements);
        *self.last_posted.lock().unwrap() = LastPosted::Unknown;
        let note = self
            .misskey
            .create_note(&NewNote {
                text,
                visibility: self.visibility,
                file_ids: Vec::new(),
            })
            .await?;
        *self.last_posted.lock().unwrap() = LastPosted::Known(Some(observation.time));
        info!(note_id = %note.id, slot = %observation.time, warnings = warnings.len(), "posted the observation");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{DateTime, FixedOffset};

    use super::*;
    use crate::jma::fake::FakeJma;
    use crate::misskey::fake::{Fail, FakeMisskey};

    /// テストの「今」。2026-10-22 21:02 JST。
    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-22T21:02:00+09:00").unwrap()
    }

    fn observation(time: &str) -> Observation {
        Observation {
            time: time.parse().unwrap(),
            temperature: amedas::Reading::Value(14.3),
            precipitation_1h: amedas::Reading::Value(0.0),
            sunshine_minutes: amedas::Reading::Value(0.0),
            wind_speed: amedas::Reading::Value(0.8),
            max_temperature: None,
            min_temperature: None,
        }
    }

    fn jma() -> FakeJma {
        FakeJma::new()
            .with_station("多治見")
            .with_observation(observation("2026-10-22T21:00:00"))
    }

    fn amedas() -> AmedasConfig {
        AmedasConfig {
            point: "52606".to_owned(),
            observation_visibility: Visibility::Home,
            min_max_visibility: Visibility::Public,
        }
    }

    fn warning() -> WarningConfig {
        WarningConfig {
            office: "210000".to_owned(),
            area: "2120400".to_owned(),
        }
    }

    fn job<'a>(
        jma: FakeJma,
        misskey: &'a FakeMisskey,
        warning: Option<&WarningConfig>,
    ) -> ObservationPoster<FakeJma, &'a FakeMisskey> {
        ObservationPoster::new(jma, misskey, &amedas(), warning, &[])
    }

    /// `time` の枠の観測値（[`observation`]）のノートの本文。
    fn note_text(time: &str) -> String {
        format!("🌡 14.3 ℃ 💨 0.8 m/s\n☀ 0 min/h 🌧 0.0 mm/h\n(多治見 {time})")
    }

    #[tokio::test]
    async fn posts_the_current_slot_once() {
        let misskey = FakeMisskey::new(now());
        let job = job(jma(), &misskey, None);
        job.run(now(), false).await.unwrap();
        job.run(now(), false).await.unwrap();

        let notes = misskey.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, note_text("21:00"));
    }

    #[tokio::test]
    async fn does_nothing_until_the_slot_is_available() {
        // 20:50 の枠は 20:57 に投稿済みで、21:00 の枠はまだ出ていない。
        let misskey =
            FakeMisskey::new(now() - chrono::TimeDelta::minutes(5)).with_note(&note_text("20:50"));
        let jma = FakeJma::new()
            .with_station("多治見")
            .with_observation(observation("2026-10-22T20:50:00"));
        job(jma, &misskey, None).run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 1);
    }

    #[tokio::test]
    async fn does_not_repost_a_slot_found_in_recent_notes() {
        // 1 分前（21:01）に投稿した 21:00 の枠が残っている。
        let misskey = FakeMisskey::new(now() - chrono::TimeDelta::minutes(1))
            .with_note("関係ないノート")
            .with_note(&note_text("21:00"));
        job(jma(), &misskey, None).run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 2);
    }

    #[tokio::test]
    async fn posts_when_recent_notes_hold_an_older_slot() {
        let misskey =
            FakeMisskey::new(now() - chrono::TimeDelta::minutes(11)).with_note(&note_text("20:50"));
        job(jma(), &misskey, None).run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 2);
    }

    #[tokio::test]
    async fn posts_a_late_previous_slot_before_the_current_one() {
        // 20:40 の枠まで投稿済み。20:50 の枠の公開が遅れ、21:00 を過ぎてから 21:00 の枠と一緒に出た。
        let misskey =
            FakeMisskey::new(now() - chrono::TimeDelta::minutes(15)).with_note(&note_text("20:40"));
        let job = job(
            jma().with_observation(observation("2026-10-22T20:50:00")),
            &misskey,
            None,
        );
        for _ in 0..3 {
            job.run(now(), false).await.unwrap();
        }
        let texts: Vec<String> = misskey.notes().into_iter().map(|note| note.text).collect();
        assert_eq!(
            texts,
            [note_text("20:40"), note_text("20:50"), note_text("21:00")]
        );
    }

    #[tokio::test]
    async fn holds_todays_slots_when_asked() {
        // 日付が変わった直後（00:02）。23:40 の枠まで投稿済みで、23:50 と 00:00 の枠が出ている。
        let midnight = DateTime::parse_from_rfc3339("2026-10-23T00:02:00+09:00").unwrap();
        let misskey = FakeMisskey::new(midnight - chrono::TimeDelta::minutes(15))
            .with_note(&note_text("23:40"));
        let job = job(
            FakeJma::new()
                .with_station("多治見")
                .with_observation(observation("2026-10-22T23:50:00"))
                .with_observation(observation("2026-10-23T00:00:00")),
            &misskey,
            None,
        );
        for _ in 0..2 {
            job.run(midnight, true).await.unwrap();
        }
        let texts: Vec<String> = misskey.notes().into_iter().map(|note| note.text).collect();
        assert_eq!(texts, [note_text("23:40"), note_text("23:50")]);
    }

    #[tokio::test]
    async fn adds_the_warning_line_and_tolerates_warning_failures() {
        let misskey = FakeMisskey::new(now());
        let kinds = vec![Kind::from_code("15").unwrap()];
        job(jma().with_warnings(kinds), &misskey, Some(&warning()))
            .run(now(), false)
            .await
            .unwrap();
        assert_eq!(
            misskey.notes()[0].text,
            "🌡 14.3 ℃ 💨 0.8 m/s\n☀ 0 min/h 🌧 0.0 mm/h\n$[bg.color=f2e700 $[fg.color=000000 強風注意報]]\n(多治見 21:00)"
        );

        // 警報の取得に失敗しても観測値は投稿する。
        let misskey = FakeMisskey::new(now());
        job(jma(), &misskey, Some(&warning()))
            .run(now(), false)
            .await
            .unwrap();
        assert_eq!(misskey.notes()[0].text, note_text("21:00"));
    }

    #[tokio::test]
    async fn fails_without_posting_when_misskey_or_the_station_is_unavailable() {
        let misskey = FakeMisskey::new(now()).failing(Fail::Read);
        let error = job(jma(), &misskey, None)
            .run(now(), false)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Misskey(_)), "{error}");
        assert!(misskey.notes().is_empty());

        // 観測値がまだ無くても、地点が引けなければエラーにする。
        let misskey = FakeMisskey::new(now());
        let jma = FakeJma::new();
        let error = job(jma, &misskey, None)
            .run(now(), false)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Jma(_)), "{error}");
        assert!(misskey.notes().is_empty());
    }

    #[tokio::test]
    async fn a_failed_post_is_retried_on_the_next_run() {
        let misskey = FakeMisskey::new(now()).failing(Fail::Create);
        let job = job(jma(), &misskey, None);
        assert!(job.run(now(), false).await.is_err());
        // 2 回目は投稿できる（同じ枠をもう一度試す）。
        misskey.recover();
        job.run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 1);
    }

    #[tokio::test]
    async fn a_post_saved_despite_an_error_is_not_repeated() {
        let misskey = FakeMisskey::new(now()).failing(Fail::CreateTimeout);
        let job = job(jma(), &misskey, None);
        assert!(job.run(now(), false).await.is_err());
        assert_eq!(misskey.notes().len(), 1);
        // 2 回目は直近のノートから同じ枠を読み戻すので、投稿しない。
        misskey.recover();
        job.run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_post_cut_off_while_waiting_is_not_repeated() {
        // scheduler の RUN_TIMEOUT と同じく、応答を待つ間に future を drop する。
        let misskey = FakeMisskey::new(now()).failing(Fail::CreateStalled);
        let job = job(jma(), &misskey, None);
        let cut_off = tokio::time::timeout(Duration::from_secs(1), job.run(now(), false)).await;
        assert!(cut_off.is_err());
        assert_eq!(misskey.notes().len(), 1);
        misskey.recover();
        job.run(now(), false).await.unwrap();
        assert_eq!(misskey.notes().len(), 1);
    }
}
