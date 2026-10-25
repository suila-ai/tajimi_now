//! 日本時間の扱い。気象庁の時刻は全て JST で、アメダスのキー（`20261022210000`）には時差の表記が無い。

use chrono::{DateTime, FixedOffset, Utc};

/// 日本標準時（UTC+9）。
pub const JST: FixedOffset = FixedOffset::east_opt(9 * 3600).unwrap();

/// 今の時刻（JST）。
pub fn now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&JST)
}
