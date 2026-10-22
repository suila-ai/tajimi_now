//! 気象庁のアメダス観測値・天気予報・警報注意報を Misskey に投稿する bot。

mod config;
mod jma;
mod jobs;
mod mfm;
mod misskey;
mod scheduler;
mod time;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use croner::Cron;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::jma::JmaClient;
use crate::jobs::{ForecastNote, MinMaxNote, ObservationPoster, PinnedNotePoster, PostingJob};
use crate::misskey::MiapiMisskey;

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(error = %error_chain(&error), "startup failed");
            ExitCode::FAILURE
        }
    }
}

/// 気象庁と Misskey を叩くときのタイムアウト。
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// 投稿を確かめる時刻。観測値は 10 分枠の数分後に出るので毎分見る。前日の気温と天気予報も同じ回に、この順で確かめる。
const CRON: &str = "0 * * * * *";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("failed to load config")]
    Config(#[from] config::Error),
    #[error("failed to build the HTTP client")]
    Http(#[from] reqwest::Error),
    #[error("failed to connect to Misskey")]
    Misskey(#[from] misskey::Error),
}

/// 設定を読み、Misskey につなぎ、停止を求められるまでジョブを回す。
async fn run() -> Result<(), Error> {
    let path = config_path();
    let config = config::Config::load(&path)?;
    info!(path = %path.display(), server = %config.misskey.server, "config loaded");

    let http = build_http_client()?;
    let misskey =
        MiapiMisskey::connect(&config.misskey.server, &config.misskey.token, http.clone()).await?;
    let jma = JmaClient::new(http);

    let job = PostingJob::new(
        config.amedas.as_ref().map(|amedas| {
            PinnedNotePoster::new(
                MinMaxNote::new(&jma, amedas, &config.chance, &config.replace),
                &misskey,
            )
        }),
        config.forecast.as_ref().map(|forecast| {
            PinnedNotePoster::new(
                ForecastNote::new(&jma, forecast, &config.chance, &config.replace),
                &misskey,
            )
        }),
        config.amedas.as_ref().map(|amedas| {
            ObservationPoster::new(
                &jma,
                &misskey,
                amedas,
                config.warning.as_ref(),
                &config.replace,
            )
        }),
    );
    let cron: Cron = CRON.parse().expect("a valid cron expression");

    let shutdown = CancellationToken::new();
    let signals = {
        let shutdown = shutdown.clone();
        async move {
            match shutdown_signal().await {
                Ok(()) => info!("shutdown requested"),
                Err(error) => error!(error = %error, "failed to listen for signals; shutting down"),
            }
            shutdown.cancel();
        }
    };

    tokio::join!(signals, scheduler::run_on_schedule(&job, &cron, &shutdown));
    info!("stopped");
    Ok(())
}

/// 設定ファイルのパス。第 1 引数で、無ければ作業ディレクトリの `config.toml`。
fn config_path() -> PathBuf {
    std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("config.toml"), PathBuf::from)
}

/// `RUST_LOG` があればそれに従い、無ければ info 以上を出す。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// 気象庁と Misskey を叩く HTTP クライアント。UA とタイムアウトを揃え、`Clone` して渡す。
fn build_http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(HTTP_TIMEOUT)
        .build()
}

/// SIGINT（Ctrl-C）か SIGTERM（`docker stop`）を待つ。
async fn shutdown_signal() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

/// エラーとその原因を `: ` でつないで 1 行にする。
pub(crate) fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cron_constant_is_valid() {
        assert!(CRON.parse::<Cron>().is_ok());
    }
}
