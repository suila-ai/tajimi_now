//! Misskey への投稿。miapi（Misskey 2025.10.0 のバインディング。2026.5.0 までのサーバを読める）を使う。

use chrono::{DateTime, Utc};
use miapi::v2025_10_0::api::notes::create::RequestVisibility;
use miapi::v2025_10_0::{Client, types};
use tracing::info;
use url::Url;

use crate::config::Visibility;

pub use miapi::Error;

/// bot が扱うノートの、必要なところだけ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    /// 本文。本文の無いノート（ファイルだけ等）は空。
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// これから作るノート。
#[derive(Debug, Clone)]
pub struct NewNote {
    pub text: String,
    pub visibility: Visibility,
    /// 添付するドライブのファイル ID。
    pub file_ids: Vec<String>,
}

/// bot が Misskey に対して行う操作。
pub trait Misskey {
    async fn create_note(&self, note: &NewNote) -> Result<Note, Error>;
    /// 自分のノートを新しい順に `limit` 件まで。リノートと他人への返信は含めない。
    async fn my_recent_notes(&self, limit: u8) -> Result<Vec<Note>, Error>;
    /// `since` より後の自分のノートを古い順に `limit` 件まで。リノートと他人への返信は含めない。
    async fn my_notes_since(&self, since: DateTime<Utc>, limit: u8) -> Result<Vec<Note>, Error>;
    async fn my_pinned_notes(&self) -> Result<Vec<Note>, Error>;
    async fn pin(&self, note_id: &str) -> Result<(), Error>;
    async fn unpin(&self, note_id: &str) -> Result<(), Error>;
}

/// 参照越しにも使えるようにする。1 つの接続を複数のジョブで共有するため。
impl<T: Misskey> Misskey for &T {
    async fn create_note(&self, note: &NewNote) -> Result<Note, Error> {
        (**self).create_note(note).await
    }

    async fn my_recent_notes(&self, limit: u8) -> Result<Vec<Note>, Error> {
        (**self).my_recent_notes(limit).await
    }

    async fn my_notes_since(&self, since: DateTime<Utc>, limit: u8) -> Result<Vec<Note>, Error> {
        (**self).my_notes_since(since, limit).await
    }

    async fn my_pinned_notes(&self) -> Result<Vec<Note>, Error> {
        (**self).my_pinned_notes().await
    }

    async fn pin(&self, note_id: &str) -> Result<(), Error> {
        (**self).pin(note_id).await
    }

    async fn unpin(&self, note_id: &str) -> Result<(), Error> {
        (**self).unpin(note_id).await
    }
}

/// miapi で Misskey を叩く実装。
#[derive(Debug)]
pub struct MiapiMisskey {
    client: Client,
    /// 自分のユーザー ID。`users/notes` で自分のノートを引くのに使う。
    user_id: String,
}

impl MiapiMisskey {
    /// `http` で接続し、`i` を呼んでトークンを確かめる。不正なトークンならここで失敗する。
    pub async fn connect(server: &Url, token: &str, http: reqwest::Client) -> Result<Self, Error> {
        let client = Client::with_http_client(http, server.as_str(), Some(token))?;
        let me = client.i().await?;
        info!(username = %me.username, user_id = %me.id, "connected to Misskey");
        Ok(Self {
            client,
            user_id: me.id,
        })
    }

    /// `users/notes` で自分のノートを引く。`since_date`（UNIX ミリ秒）を渡すと、Misskey はその時刻より後のノートを古い順に返す。
    async fn my_notes(&self, since_date: Option<i64>, limit: u8) -> Result<Vec<Note>, Error> {
        let notes = self
            .client
            .users()
            .notes()
            .user_id(&self.user_id)
            .with_replies(false)
            .with_renotes(false)
            .maybe_since_date(since_date)
            .limit(limit)
            .await?;
        Ok(notes.into_iter().map(Note::from).collect())
    }
}

impl Misskey for MiapiMisskey {
    async fn create_note(&self, note: &NewNote) -> Result<Note, Error> {
        // `specified` は宛先（visibleUserIds）を送らない。自分だけが読めるノートになる。
        let visibility = match note.visibility {
            Visibility::Public => RequestVisibility::Public,
            Visibility::Home => RequestVisibility::Home,
            Visibility::Followers => RequestVisibility::Followers,
            Visibility::Specified => RequestVisibility::Specified,
        };
        let file_ids = (!note.file_ids.is_empty()).then(|| note.file_ids.clone());
        let response = self
            .client
            .notes()
            .create()
            .visibility(visibility)
            .text(&note.text)
            .maybe_file_ids(file_ids)
            .await?;
        Ok(response.created_note.into())
    }

    async fn my_recent_notes(&self, limit: u8) -> Result<Vec<Note>, Error> {
        self.my_notes(None, limit).await
    }

    async fn my_notes_since(&self, since: DateTime<Utc>, limit: u8) -> Result<Vec<Note>, Error> {
        self.my_notes(Some(since.timestamp_millis()), limit).await
    }

    async fn my_pinned_notes(&self) -> Result<Vec<Note>, Error> {
        let me = self.client.i().await?;
        Ok(me.pinned_notes.into_iter().map(Note::from).collect())
    }

    async fn pin(&self, note_id: &str) -> Result<(), Error> {
        self.client.i().pin().note_id(note_id).await?;
        Ok(())
    }

    async fn unpin(&self, note_id: &str) -> Result<(), Error> {
        self.client.i().unpin().note_id(note_id).await?;
        Ok(())
    }
}

/// miapi の `Note` から、bot が使うところだけを取り出す。
impl From<types::Note> for Note {
    fn from(note: types::Note) -> Self {
        Self {
            id: note.id,
            text: note.text.unwrap_or_default(),
            created_at: note.created_at,
        }
    }
}

/// ジョブのテストで使う、メモリ上の Misskey。
#[cfg(test)]
pub mod fake {
    use std::sync::Mutex;

    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};

    use super::{Error, Misskey, NewNote, Note};

    /// 失敗させたい操作。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fail {
        Create,
        /// ノートは作られたが、応答がエラーになる（タイムアウト等）。
        CreateTimeout,
        /// ノートは作られたが、応答が返らない。待つ側が打ち切るまで止まる。
        CreateStalled,
        /// `i`（ピン留めの一覧）と `users/notes`。
        Read,
        Pin,
    }

    #[derive(Debug, Default)]
    struct State {
        notes: Vec<Note>,
        /// ノートごとの添付ファイル ID。
        file_ids: Vec<(String, Vec<String>)>,
        /// ピン留めしたノートの id。ピン留めした順。
        pinned: Vec<String>,
        fail: Option<Fail>,
        /// ピン留めの上限（Misskey のロールの pinLimit）。無ければ無制限。
        pin_limit: Option<usize>,
        /// 作成日時の起点。
        base_time: DateTime<Utc>,
    }

    impl State {
        /// ノートを足す。作成日時は、足した順に 1 分ずつ進む。
        fn push(&mut self, text: &str, file_ids: &[String]) -> Note {
            let index = self.notes.len();
            let note = Note {
                id: format!("note{index}"),
                text: text.to_owned(),
                created_at: self.base_time + TimeDelta::minutes(index as i64),
            };
            self.notes.push(note.clone());
            self.file_ids.push((note.id.clone(), file_ids.to_vec()));
            note
        }
    }

    fn api_error(endpoint: &str, code: &str) -> Error {
        Error::Api(Box::new(miapi::ApiError {
            status: 400,
            endpoint: endpoint.to_owned(),
            id: "00000000-0000-0000-0000-000000000000".to_owned(),
            code: code.to_owned(),
            message: format!("fake error {code}"),
            kind: None,
            info: None,
        }))
    }

    #[derive(Debug)]
    pub struct FakeMisskey {
        state: Mutex<State>,
    }

    impl FakeMisskey {
        /// 作成日時が `base_time` から 1 分ずつ進む Misskey。
        pub fn new(base_time: DateTime<FixedOffset>) -> Self {
            Self {
                state: Mutex::new(State {
                    base_time: base_time.to_utc(),
                    ..State::default()
                }),
            }
        }

        /// 既にあるノートを足す。
        pub fn with_note(mut self, text: &str) -> Self {
            self.state.get_mut().unwrap().push(text, &[]);
            self
        }

        /// 既にあるノートを足して、ピン留めしておく。
        pub fn with_pinned_note(mut self, text: &str) -> Self {
            let state = self.state.get_mut().unwrap();
            let id = state.push(text, &[]).id;
            state.pinned.push(id);
            self
        }

        /// 指定した操作を失敗させる。
        pub fn failing(self, fail: Fail) -> Self {
            self.state.lock().unwrap().fail = Some(fail);
            self
        }

        /// 失敗させるのをやめる。
        pub fn recover(&self) {
            self.state.lock().unwrap().fail = None;
        }

        /// ピン留めの上限を決める。
        pub fn with_pin_limit(mut self, limit: usize) -> Self {
            self.state.get_mut().unwrap().pin_limit = Some(limit);
            self
        }

        /// すべてのノート。作成順。
        pub fn notes(&self) -> Vec<Note> {
            self.state.lock().unwrap().notes.clone()
        }

        /// `note_id` の添付ファイル ID。
        pub fn file_ids_of(&self, note_id: &str) -> Vec<String> {
            let state = self.state.lock().unwrap();
            state
                .file_ids
                .iter()
                .find(|(id, _)| id == note_id)
                .map(|(_, file_ids)| file_ids.clone())
                .unwrap_or_default()
        }

        /// ピン留めしているノートの id。ピン留めした順。
        pub fn pinned_ids(&self) -> Vec<String> {
            self.state.lock().unwrap().pinned.clone()
        }
    }

    impl Misskey for FakeMisskey {
        async fn create_note(&self, note: &NewNote) -> Result<Note, Error> {
            if self.state.lock().unwrap().fail == Some(Fail::CreateStalled) {
                self.state.lock().unwrap().push(&note.text, &note.file_ids);
                return std::future::pending().await;
            }
            let mut state = self.state.lock().unwrap();
            match state.fail {
                Some(Fail::Create) => {
                    return Err(api_error("notes/create", "RATE_LIMIT_EXCEEDED"));
                }
                Some(Fail::CreateTimeout) => {
                    state.push(&note.text, &note.file_ids);
                    return Err(api_error("notes/create", "INTERNAL_ERROR"));
                }
                _ => {}
            }
            Ok(state.push(&note.text, &note.file_ids))
        }

        async fn my_recent_notes(&self, limit: u8) -> Result<Vec<Note>, Error> {
            if self.state.lock().unwrap().fail == Some(Fail::Read) {
                return Err(api_error("users/notes", "INTERNAL_ERROR"));
            }
            let mut notes = self.notes();
            notes.sort_by_key(|note| std::cmp::Reverse(note.created_at));
            notes.truncate(usize::from(limit));
            Ok(notes)
        }

        async fn my_notes_since(
            &self,
            since: DateTime<Utc>,
            limit: u8,
        ) -> Result<Vec<Note>, Error> {
            if self.state.lock().unwrap().fail == Some(Fail::Read) {
                return Err(api_error("users/notes", "INTERNAL_ERROR"));
            }
            let mut notes = self.notes();
            notes.retain(|note| note.created_at > since);
            notes.sort_by_key(|note| note.created_at);
            notes.truncate(usize::from(limit));
            Ok(notes)
        }

        async fn my_pinned_notes(&self) -> Result<Vec<Note>, Error> {
            let state = self.state.lock().unwrap();
            if state.fail == Some(Fail::Read) {
                return Err(api_error("i", "INTERNAL_ERROR"));
            }
            Ok(state
                .pinned
                .iter()
                .filter_map(|id| state.notes.iter().find(|n| &n.id == id).cloned())
                .collect())
        }

        async fn pin(&self, note_id: &str) -> Result<(), Error> {
            let mut state = self.state.lock().unwrap();
            if state.fail == Some(Fail::Pin) {
                return Err(api_error("i/pin", "INTERNAL_ERROR"));
            }
            if state
                .pin_limit
                .is_some_and(|limit| state.pinned.len() >= limit)
            {
                return Err(api_error("i/pin", "PIN_LIMIT_EXCEEDED"));
            }
            if !state.notes.iter().any(|n| n.id == note_id) {
                return Err(api_error("i/pin", "NO_SUCH_NOTE"));
            }
            if state.pinned.iter().any(|id| id == note_id) {
                return Err(api_error("i/pin", "ALREADY_PINNED"));
            }
            state.pinned.push(note_id.to_owned());
            Ok(())
        }

        async fn unpin(&self, note_id: &str) -> Result<(), Error> {
            let mut state = self.state.lock().unwrap();
            let before = state.pinned.len();
            state.pinned.retain(|id| id != note_id);
            if state.pinned.len() == before {
                return Err(api_error("i/unpin", "NO_SUCH_NOTE"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{body_json, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const NOTE: &str = include_str!("../tests/fixtures/misskey/note.json");
    const ME: &str = include_str!("../tests/fixtures/misskey/i.json");

    fn note_json() -> Value {
        serde_json::from_str(NOTE).unwrap()
    }

    fn me_json() -> Value {
        serde_json::from_str(ME).unwrap()
    }

    /// `i` に応答するモックサーバと、それにつないだクライアント。
    async fn connected() -> (MockServer, MiapiMisskey) {
        let server = MockServer::start().await;
        Mock::given(path("/api/i"))
            .and(body_json(json!({"i": "tok"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_json()))
            .mount(&server)
            .await;
        let misskey = MiapiMisskey::connect(
            &server.uri().parse().unwrap(),
            "tok",
            reqwest::Client::new(),
        )
        .await
        .unwrap();
        (server, misskey)
    }

    #[tokio::test]
    async fn connect_verifies_the_token_and_remembers_the_user_id() {
        let (_server, misskey) = connected().await;
        assert_eq!(misskey.user_id, me_json()["id"]);
    }

    #[tokio::test]
    async fn connect_fails_with_a_bad_token() {
        let server = MockServer::start().await;
        Mock::given(path("/api/i"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {"message": "Authentication failed.", "code": "AUTHENTICATION_FAILED", "id": "b0a7f5f8-dc2f-4171-b91f-de88ad238e14"}
            })))
            .mount(&server)
            .await;
        let error = MiapiMisskey::connect(
            &server.uri().parse().unwrap(),
            "bad",
            reqwest::Client::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, Error::Api(ref e) if e.code == "AUTHENTICATION_FAILED"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn create_note_sends_visibility_text_and_token() {
        let (server, misskey) = connected().await;
        Mock::given(path("/api/notes/create"))
            .and(body_json(
                json!({"visibility": "home", "text": "hello", "i": "tok"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"createdNote": note_json()})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let note = misskey
            .create_note(&NewNote {
                text: "hello".to_owned(),
                visibility: Visibility::Home,
                file_ids: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(note.id, note_json()["id"]);
        assert_eq!(
            note.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            note_json()["createdAt"]
        );
    }

    #[tokio::test]
    async fn specified_sends_no_visible_user_ids_and_file_ids_are_attached() {
        let (server, misskey) = connected().await;
        Mock::given(path("/api/notes/create"))
            .and(body_json(
                json!({"visibility": "specified", "text": "hello", "fileIds": ["f1", "f2"], "i": "tok"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"createdNote": note_json()})),
            )
            .expect(1)
            .mount(&server)
            .await;
        misskey
            .create_note(&NewNote {
                text: "hello".to_owned(),
                visibility: Visibility::Specified,
                file_ids: vec!["f1".to_owned(), "f2".to_owned()],
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn recent_notes_query_own_notes_without_replies_and_renotes() {
        let (server, misskey) = connected().await;
        Mock::given(path("/api/users/notes"))
            .and(body_json(json!({
                "userId": me_json()["id"], "withReplies": false, "withRenotes": false, "limit": 10, "i": "tok"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([note_json()])))
            .expect(1)
            .mount(&server)
            .await;
        let notes = misskey.my_recent_notes(10).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, note_json()["id"]);
    }

    #[tokio::test]
    async fn notes_since_send_the_date_in_milliseconds() {
        let (server, misskey) = connected().await;
        Mock::given(path("/api/users/notes"))
            .and(body_json(json!({
                "userId": me_json()["id"], "withReplies": false, "withRenotes": false,
                "sinceDate": 1_792_681_200_000_i64, "limit": 100, "i": "tok"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([note_json()])))
            .expect(1)
            .mount(&server)
            .await;
        // 2026-10-23 00:00 JST。
        let since = "2026-10-22T15:00:00Z".parse().unwrap();
        let notes = misskey.my_notes_since(since, 100).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, note_json()["id"]);
    }

    #[tokio::test]
    async fn pinned_notes_come_from_i() {
        let (_server, misskey) = connected().await;
        let notes = misskey.my_pinned_notes().await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, me_json()["pinnedNotes"][0]["id"]);
    }

    #[tokio::test]
    async fn pin_and_unpin_send_the_note_id() {
        let (server, misskey) = connected().await;
        for endpoint in ["/api/i/pin", "/api/i/unpin"] {
            Mock::given(path(endpoint))
                .and(body_json(json!({"noteId": "n1", "i": "tok"})))
                .respond_with(ResponseTemplate::new(200).set_body_json(me_json()))
                .expect(1)
                .mount(&server)
                .await;
        }
        misskey.pin("n1").await.unwrap();
        misskey.unpin("n1").await.unwrap();
    }

    #[tokio::test]
    async fn api_errors_carry_the_code() {
        let (server, misskey) = connected().await;
        Mock::given(path("/api/i/pin"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {"message": "You can not pin notes any more.", "code": "PIN_LIMIT_EXCEEDED", "id": "15a018eb-58e5-4da1-93be-330fcc5e4e1a"}
            })))
            .mount(&server)
            .await;
        let error = misskey.pin("n1").await.unwrap_err();
        assert!(
            matches!(error, Error::Api(ref e) if e.code == "PIN_LIMIT_EXCEEDED"),
            "{error}"
        );
    }

    /// 実サーバに対する往復。`scripts/misskey/up.sh` で立てたローカル Misskey に向けて手で実行する。
    #[tokio::test]
    #[ignore = "needs a live server: set MISSKEY_SERVER and MISSKEY_TOKEN (see scripts/misskey/up.sh)"]
    async fn live_round_trip() {
        let server: Url = std::env::var("MISSKEY_SERVER")
            .expect("MISSKEY_SERVER を設定する")
            .parse()
            .unwrap();
        let token = std::env::var("MISSKEY_TOKEN").expect("MISSKEY_TOKEN を設定する");
        let misskey = MiapiMisskey::connect(&server, &token, reqwest::Client::new())
            .await
            .unwrap();

        // ノートの作成と削除はタイムラインへ非同期に反映され、削除には 1 秒の間隔の制限もあるので、その都度待つ。
        let settle = || tokio::time::sleep(std::time::Duration::from_secs(1));

        let text = format!("live_round_trip {}", Utc::now().to_rfc3339());
        let note = misskey
            .create_note(&NewNote {
                text: text.clone(),
                visibility: Visibility::Specified,
                file_ids: Vec::new(),
            })
            .await
            .unwrap();
        let contains = |notes: Vec<Note>| notes.iter().any(|n| n.id == note.id);

        settle().await;
        let recent = misskey.my_recent_notes(10).await.unwrap();
        let read_back = recent
            .iter()
            .find(|n| n.id == note.id)
            .expect("the note is read back");
        assert_eq!(read_back.text, text);
        // `sinceDate` はミリ秒で、その時刻より後のノートだけが返る。
        let second = chrono::TimeDelta::seconds(1);
        assert!(contains(
            misskey
                .my_notes_since(note.created_at - second, 10)
                .await
                .unwrap()
        ));
        assert!(!contains(
            misskey
                .my_notes_since(note.created_at + second, 10)
                .await
                .unwrap()
        ));

        misskey.pin(&note.id).await.unwrap();
        assert!(contains(misskey.my_pinned_notes().await.unwrap()));
        misskey.unpin(&note.id).await.unwrap();
        assert!(!contains(misskey.my_pinned_notes().await.unwrap()));
        misskey
            .client
            .notes()
            .delete()
            .note_id(&note.id)
            .await
            .unwrap();
        settle().await;
        assert!(!contains(misskey.my_recent_notes(10).await.unwrap()));
    }
}
