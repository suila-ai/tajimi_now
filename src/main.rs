//! 気象庁のアメダス観測値・天気予報・警報注意報を Misskey に投稿する bot。

// TODO(wiring): ジョブを配線するまで、設定の項目が未使用になる。配線したら外す。
#![allow(dead_code)]

mod config;
mod jma;
mod mfm;
mod misskey;
mod time;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;

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

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("failed to load config")]
    Config(#[from] config::Error),
    #[error("failed to build the HTTP client")]
    Http(#[from] reqwest::Error),
}

/// 設定を読み、Misskey につなぎ、停止を求められるまでジョブを回す。
async fn run() -> Result<(), Error> {
    let path = config_path();
    let config = config::Config::load(&path)?;
    info!(path = %path.display(), server = %config.misskey.server, "config loaded");
    let _http = build_http_client()?;
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
