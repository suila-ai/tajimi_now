//! ジョブ。気象庁から取ったものを Misskey に反映する「判断」の層。

pub mod observation;
pub mod pinned;
pub mod posting;

pub use observation::ObservationPoster;
pub use pinned::{ForecastNote, MinMaxNote, PinnedNotePoster};
pub use posting::PostingJob;

/// scheduler が定期に実行する 1 つの仕事。
pub trait Job {
    /// ログに出す名前。
    fn name(&self) -> &'static str;
    async fn run(&self) -> Result<(), Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Jma(#[from] crate::jma::Error),
    #[error(transparent)]
    Misskey(#[from] crate::misskey::Error),
}
