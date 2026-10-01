//! The originating forge target, separate from Comparison identity and credentials.
use crate::domain::{Comparison, Oid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReviewOrigin {
    #[default]
    Local,
    GitLab {
        base_url: String,
        project: String,
        iid: u64,
        base_sha: String,
        start_sha: Option<String>,
        head_sha: String,
    },
}

impl ReviewOrigin {
    fn target_identity(&self) -> Option<(&str, &str, u64)> {
        match self {
            Self::Local => None,
            Self::GitLab {
                base_url,
                project,
                iid,
                ..
            } => Some((base_url, project, *iid)),
        }
    }
    pub fn same_target(&self, other: &Self) -> bool {
        self.target_identity() == other.target_identity()
    }

    pub fn preparation_eligibility(&self, comparison: &Comparison) -> Result<(), &'static str> {
        let Self::GitLab {
            base_url,
            project,
            iid,
            base_sha,
            head_sha,
            ..
        } = self
        else {
            return Err("Local Review · publication unavailable");
        };
        if base_url.is_empty()
            || project.is_empty()
            || *iid == 0
            || base_sha.parse::<Oid>().is_err()
            || head_sha.parse::<Oid>().is_err()
        {
            return Err("MR version information incomplete · publication unavailable");
        }
        if comparison.uncommitted
            || comparison.base_oid != base_sha.parse().ok()
            || Some(comparison.head_oid) != head_sha.parse().ok()
        {
            return Err("MR commit subset · publication unavailable");
        }
        Ok(())
    }
    /// Only the complete, actually reviewed MR diff pair is publishable.
    pub fn eligibility(&self, comparison: &Comparison) -> Result<(), &'static str> {
        match self {
            Self::Local => Err("Local Review · publication unavailable"),
            Self::GitLab {
                base_url,
                project,
                iid,
                base_sha,
                start_sha,
                head_sha,
            } => {
                if base_url.is_empty()
                    || project.is_empty()
                    || *iid == 0
                    || base_sha.parse::<Oid>().is_err()
                    || head_sha.parse::<Oid>().is_err()
                    || start_sha
                        .as_deref()
                        .is_none_or(|sha| sha.parse::<Oid>().is_err())
                {
                    return Err("MR version information incomplete · publication unavailable");
                }
                if comparison.uncommitted
                    || comparison.base_oid != base_sha.parse().ok()
                    || Some(comparison.head_oid) != head_sha.parse().ok()
                {
                    return Err("MR commit subset · publication unavailable");
                }
                Ok(())
            }
        }
    }
}

use crate::domain::{DraftComment, Review};
use crate::export::ReviewContexts;
use crate::gitlab_publication::GitLabPublication;
use anyhow::{Context, Result, bail};
use gpui_http_client::HttpClient;
use std::{path::PathBuf, sync::Arc};

/// Credentials belong to the live connection, never to durable Review data.
pub struct Connection {
    pub base_url: String,
    pub pat: String,
}
impl Connection {
    pub fn new(base_url: String, pat: String) -> Self {
        Self { base_url, pat }
    }
    pub fn matches(&self, origin: &ReviewOrigin) -> bool {
        let ReviewOrigin::GitLab { base_url, .. } = origin else {
            return false;
        };
        let safe_url = |raw: &str| {
            url::Url::parse(raw).ok().filter(|url| {
                matches!(url.scheme(), "https" | "http")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
        };
        match (safe_url(&self.base_url), safe_url(base_url)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffPosition {
    pub position_type: String,
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
    pub old_path: String,
    pub new_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReceipt {
    pub discussion_id: String,
    pub note_id: u64,
    pub confirmed_body: String,
    pub remote_position: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicationState {
    Published(PublicationReceipt),
    Failed(String),
    Unknown(String),
}
impl PublicationState {
    pub fn label(&self) -> String {
        match self {
            Self::Published(_) => "Published on GitLab".into(),
            Self::Failed(message) => format!("Publish failed: {message}"),
            Self::Unknown(message) => format!("Publication result unknown: {message}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationRecord {
    pub operation_id: String,
    pub body: String,
    pub author_id: Option<u64>,
    pub position: Option<DiffPosition>,
    pub state: PublicationState,
}
impl PublicationRecord {
    pub fn marker(&self) -> String {
        format!("<!-- reviewfox:operation={} -->", self.operation_id)
    }
    pub fn remote_body(&self) -> String {
        format!("{}\n\n{}", self.body, self.marker())
    }
}

#[derive(Clone)]
pub struct PublicationStore {
    directory: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct StoredPublication {
    version: u32,
    comparison: Comparison,
    origin: ReviewOrigin,
    comment_id: u64,
    record: PublicationRecord,
}
impl PublicationStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn application() -> Result<Self> {
        Ok(Self::new(
            dirs::data_dir()
                .context("Publication storage directory unavailable")?
                .join("ReviewFox")
                .join("publications"),
        ))
    }
    fn path(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        comment_id: u64,
    ) -> Result<PathBuf> {
        let bytes =
            serde_json::to_vec(&(comparison, "gitlab", origin.target_identity(), comment_id))?;
        let hash = git2::Oid::hash_object(git2::ObjectType::Blob, &bytes)?;
        Ok(self.directory.join(format!("{hash}.json")))
    }
    pub fn load(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        comment_id: u64,
    ) -> Result<Option<PublicationRecord>> {
        let path = self.path(comparison, origin, comment_id)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context("Cannot read Publication storage"),
        };
        let saved: StoredPublication = serde_json::from_slice(&bytes)
            .with_context(|| format!("Unreadable Publication preserved at {}", path.display()))?;
        if saved.version != 1
            || saved.comparison != *comparison
            || !saved.origin.same_target(origin)
            || saved.comment_id != comment_id
            || uuid::Uuid::parse_str(&saved.record.operation_id).is_err()
        {
            bail!(
                "Unsupported or mismatched Publication preserved at {}",
                path.display()
            );
        }
        Ok(Some(saved.record))
    }
    pub fn save(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        comment_id: u64,
        record: &PublicationRecord,
    ) -> Result<()> {
        self.load(comparison, origin, comment_id)?;
        std::fs::create_dir_all(&self.directory)?;
        let path = self.path(comparison, origin, comment_id)?;
        let saved = StoredPublication {
            version: 1,
            comparison: comparison.clone(),
            origin: origin.clone(),
            comment_id,
            record: record.clone(),
        };
        crate::review_store::atomic_save(&path, &serde_json::to_vec(&saved)?)
    }
}

#[derive(Clone)]
pub struct PublicationService {
    adapter: GitLabPublication,
    pub store: PublicationStore,
}
impl PublicationService {
    pub fn new(http: Arc<dyn HttpClient>, store: PublicationStore) -> Self {
        Self {
            adapter: GitLabPublication::new(http),
            store,
        }
    }
    pub async fn publish(
        &self,
        review: &Review,
        contexts: &ReviewContexts,
        origin: &ReviewOrigin,
        comment_id: u64,
        connection: &Connection,
    ) -> Result<PublicationRecord> {
        let previous = self.store.load(&review.comparison, origin, comment_id)?;
        if let Some(record) = &previous {
            match record.state {
                PublicationState::Published(_) => return Ok(record.clone()),
                PublicationState::Unknown(_) => {
                    bail!("Result unknown; check GitLab before another create")
                }
                PublicationState::Failed(_) => {}
            }
        }
        origin
            .preparation_eligibility(&review.comparison)
            .map_err(anyhow::Error::msg)?;
        let comment: &DraftComment = review
            .comments
            .iter()
            .find(|comment| comment.id == comment_id)
            .context("DraftComment no longer exists")?;
        let mut record = PublicationRecord {
            operation_id: previous
                .as_ref()
                .map(|r| r.operation_id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            body: comment.body.clone(),
            author_id: previous.and_then(|r| r.author_id),
            position: None,
            state: PublicationState::Unknown(
                "Confirmation unavailable; check GitLab before publishing again".into(),
            ),
        };
        self.store
            .save(&review.comparison, origin, comment_id, &record)?;
        if !connection.matches(origin) || connection.pat.trim().is_empty() {
            record.state = PublicationState::Failed(if connection.pat.trim().is_empty() {
                "No GitLab token configured; add a token with api scope".into()
            } else {
                "Current connection does not match the Review's GitLab host; reconnect before retrying".into()
            });
            self.store
                .save(&review.comparison, origin, comment_id, &record)?;
            return Ok(record);
        }
        let author = self.adapter.author(origin, connection).await;
        match author {
            Ok(author) if record.author_id.is_none_or(|expected| expected == author) => {
                record.author_id = Some(author);
                self.store
                    .save(&review.comparison, origin, comment_id, &record)?;
            }
            Ok(_) => {
                record.state = PublicationState::Failed(
                    "GitLab account changed; reconnect the original account before retrying".into(),
                );
                self.store
                    .save(&review.comparison, origin, comment_id, &record)?;
                return Ok(record);
            }
            Err(message) => {
                record.state = PublicationState::Failed(message);
                self.store
                    .save(&review.comparison, origin, comment_id, &record)?;
                return Ok(record);
            }
        }
        match self
            .adapter
            .prepare(
                origin,
                comment,
                contexts.get(&comment_id).map(Arc::as_ref),
                connection,
            )
            .await
        {
            Ok(position) => record.position = Some(position),
            Err(message) => {
                record.state = PublicationState::Failed(message);
                self.store
                    .save(&review.comparison, origin, comment_id, &record)?;
                return Ok(record);
            }
        }
        self.store
            .save(&review.comparison, origin, comment_id, &record)?;
        record.state = self.adapter.create(origin, connection, &record).await;
        self.store
            .save(&review.comparison, origin, comment_id, &record)?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Oid, Repository, Side};
    use crate::review_store::ReviewStore;
    use crate::ui::diff::review::OpenReview;
    use futures::AsyncReadExt;
    use gpui_http_client::{AsyncBody, FakeHttpClient, http};
    fn comparison() -> Comparison {
        Comparison {
            repository: Repository::new("/tmp/publication-test".into()),
            base_oid: Some(Oid::from_bytes([1; 20])),
            head_oid: Oid::from_bytes([2; 20]),
            uncommitted: false,
        }
    }
    fn origin() -> ReviewOrigin {
        ReviewOrigin::GitLab {
            base_url: "https://gitlab.example.com".into(),
            project: "team/repo".into(),
            iid: 10,
            base_sha: Oid::from_bytes([1; 20]).to_string(),
            head_sha: Oid::from_bytes([2; 20]).to_string(),
            start_sha: Some(Oid::from_bytes([3; 20]).to_string()),
        }
    }
    fn version() -> serde_json::Value {
        serde_json::json!({"id":7,"base_commit_sha":Oid::from_bytes([1;20]).to_string(),"head_commit_sha":Oid::from_bytes([2;20]).to_string(),"start_commit_sha":Oid::from_bytes([3;20]).to_string(),"diffs":[{"old_path":"a.rs","new_path":"a.rs","diff":"@@ -1,2 +1,3 @@\n unchanged\n+added\n tail\n"}]})
    }
    fn drafted(directory: &std::path::Path, side: Side, line: u32) -> OpenReview {
        let mut open = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.join("local")),
        );
        open.begin_draft(side, line, 1);
        open.capture_draft_context(
            "unchanged\ntail\n".into(),
            "unchanged\nadded\ntail\n".into(),
        );
        open.commit("please explain");
        open
    }
    fn response(value: serde_json::Value) -> http::Response<AsyncBody> {
        http::Response::builder()
            .status(200)
            .body(AsyncBody::from(value.to_string()))
            .unwrap()
    }
    fn preflight(path: &str) -> Option<serde_json::Value> {
        if path.ends_with("/user") {
            Some(serde_json::json!({"id":42}))
        } else if path.ends_with("/versions") {
            Some(serde_json::json!([version()]))
        } else if path.ends_with("/versions/7") {
            Some(version())
        } else {
            None
        }
    }

    #[test]
    fn permission_failures_remain_retryable_but_lost_and_server_error_responses_do_not() {
        for status in [403, 422, 500, 0] {
            let directory = tempfile::tempdir().unwrap();
            let open = drafted(directory.path(), Side::Postimage, 2);
            let store = PublicationStore::new(directory.path().join("remote"));
            let observer = store.clone();
            let client = FakeHttpClient::create(move |request| {
                let observer = observer.clone();
                async move {
                    if let Some(value) = preflight(request.uri().path()) {
                        return Ok(response(value));
                    }
                    let record = observer.load(&comparison(), &origin(), 1).unwrap().unwrap();
                    assert!(matches!(record.state, PublicationState::Unknown(_)));
                    assert_eq!(record.author_id, Some(42));
                    assert!(record.position.is_some());
                    if status == 0 {
                        return Err(anyhow::anyhow!("transport secret must never appear"));
                    }
                    Ok(http::Response::builder()
                        .status(status)
                        .body(AsyncBody::from("secret server details"))
                        .unwrap())
                }
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            let result = futures::executor::block_on(service.publish(
                open.review(),
                open.contexts(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
            if status == 403 || status == 422 {
                assert!(matches!(result.state, PublicationState::Failed(_)));
            } else {
                assert!(matches!(result.state, PublicationState::Unknown(_)));
                let no_network = FakeHttpClient::create(|_| async {
                    panic!("unknown result must never resend")
                });
                let reopened = PublicationService::new(no_network, store.clone());
                assert!(
                    futures::executor::block_on(reopened.publish(
                        open.review(),
                        open.contexts(),
                        &origin(),
                        1,
                        &connection
                    ))
                    .is_err()
                );
            }
            assert!(!result.state.label().contains("secret"));
            assert_eq!(
                store.load(&comparison(), &origin(), 1).unwrap(),
                Some(result)
            );
            assert_eq!(open.current().comments[0].body, "please explain");
        }
    }

    #[test]
    fn cancelling_a_sent_create_leaves_unknown_intent_for_reopen() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        let client = FakeHttpClient::create(|request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            futures::future::pending().await
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        assert!(
            service
                .publish(open.review(), open.contexts(), &origin(), 1, &connection)
                .now_or_never()
                .is_none()
        );
        let record = store.load(&comparison(), &origin(), 1).unwrap().unwrap();
        assert!(matches!(record.state, PublicationState::Unknown(_)));
        assert!(record.position.is_some());
        assert_eq!(record.author_id, Some(42));
    }

    #[test]
    fn failed_create_pins_its_account_and_other_host_connections_send_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        let client = FakeHttpClient::create(|request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            Ok(http::Response::builder()
                .status(403)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let failed = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        assert!(matches!(failed.state, PublicationState::Failed(_)));
        let changed_account = FakeHttpClient::create(|request| async move {
            assert!(request.uri().path().ends_with("/user"));
            Ok(response(serde_json::json!({"id":99})))
        });
        let changed = PublicationService::new(changed_account, store.clone());
        let failed = futures::executor::block_on(changed.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        assert!(failed.state.label().contains("account changed"));
        assert_eq!(failed.author_id, Some(42));
        let other_host =
            Connection::new("https://another.example.com".into(), "other secret".into());
        let no_network =
            FakeHttpClient::create(|_| async { panic!("wrong host must send no credentials") });
        let service = PublicationService::new(no_network, store);
        let failed = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &other_host,
        ))
        .unwrap();
        assert!(failed.state.label().contains("host"));
    }

    #[test]
    fn unchanged_lines_use_both_raw_patch_coordinates_and_targets_keep_separate_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let open = drafted(directory.path(), Side::Postimage, 3);
        let client = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(payload["position"]["old_line"], 2);
            assert_eq!(payload["position"]["new_line"], 3);
            Ok(response(
                serde_json::json!({"id":"discussion-2","notes":[{"id":52,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
            ))
        });
        let store = PublicationStore::new(directory.path().join("remote"));
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let result = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        assert!(matches!(result.state, PublicationState::Published(_)));
        let mut other = origin();
        if let ReviewOrigin::GitLab { iid, .. } = &mut other {
            *iid = 11;
        }
        assert!(store.load(&comparison(), &other, 1).unwrap().is_none());
    }

    #[test]
    fn deleted_line_uses_old_coordinate_and_missing_start_metadata_does_not_duplicate_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let mut incomplete = origin();
        if let ReviewOrigin::GitLab { start_sha, .. } = &mut incomplete {
            *start_sha = None;
        }
        let mut open = OpenReview::reopen(
            comparison(),
            "a.rs",
            incomplete.clone(),
            ReviewStore::new(directory.path().join("local")),
        );
        open.begin_draft(Side::Preimage, 2, 1);
        open.capture_draft_context(
            "unchanged\nremoved\ntail\n".into(),
            "unchanged\ntail\n".into(),
        );
        open.commit("removed comment");
        let client = FakeHttpClient::create(|mut request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            let mut removed = version();
            removed["diffs"][0]["diff"] = "@@ -1,3 +1,2 @@\n unchanged\n-removed\n tail\n".into();
            if request.uri().path().ends_with("/versions") {
                return Ok(response(serde_json::json!([removed])));
            }
            if request.uri().path().ends_with("/versions/7") {
                return Ok(response(removed));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(payload["position"]["old_line"], 2);
            assert!(payload["position"].get("new_line").is_none());
            assert_eq!(
                payload["position"]["start_sha"],
                Oid::from_bytes([3; 20]).to_string()
            );
            Ok(response(
                serde_json::json!({"id":"discussion-removed","notes":[{"id":53,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
            ))
        });
        let store = PublicationStore::new(directory.path().join("remote"));
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let result = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &incomplete,
            1,
            &connection,
        ))
        .unwrap();
        assert!(matches!(result.state, PublicationState::Published(_)));
        assert_eq!(
            store.load(&comparison(), &origin(), 1).unwrap(),
            Some(result)
        );
    }

    #[test]
    fn expanded_unchanged_line_requires_matching_captured_text_on_both_sides() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        );
        open.begin_draft(Side::Postimage, 4, 1);
        open.capture_draft_context(
            "unchanged\ntail\ndifferent\n".into(),
            "unchanged\nadded\ntail\nselected\n".into(),
        );
        open.commit("comment");
        let client = FakeHttpClient::create(|request| async move {
            let response = if request.uri().path().ends_with("/user") {
                serde_json::json!({"id":42})
            } else if request.uri().path().ends_with("/versions") {
                serde_json::json!([version()])
            } else if request.uri().path().ends_with("/versions/7") {
                version()
            } else {
                panic!("unverified expanded context must not be sent")
            };
            Ok(http::Response::builder()
                .status(200)
                .body(AsyncBody::from(response.to_string()))
                .unwrap())
        });
        let service = PublicationService::new(
            client,
            PublicationStore::new(directory.path().join("remote")),
        );
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let record = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        assert!(matches!(record.state, PublicationState::Failed(_)));
    }

    #[test]
    fn explicit_publish_creates_single_line_with_marker_and_reopens_without_duplicate() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        );
        open.begin_draft(Side::Postimage, 2, 1);
        open.capture_draft_context(
            "unchanged\ntail\n".into(),
            "unchanged\nadded\ntail\n".into(),
        );
        open.commit("please explain");
        let client = FakeHttpClient::create(|mut request| async move {
            assert!(matches!(
                request
                    .extensions()
                    .get::<gpui_http_client::RedirectPolicy>(),
                Some(gpui_http_client::RedirectPolicy::NoFollow)
            ));
            let path = request.uri().path();
            let response = if path.ends_with("/user") {
                serde_json::json!({"id":42,"username":"alice"})
            } else if path.ends_with("/versions") {
                serde_json::json!([version()])
            } else if path.ends_with("/versions/7") {
                version()
            } else {
                assert_eq!(request.method(), http::Method::POST);
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(payload["position"]["new_line"], 2);
                assert!(payload["position"].get("old_line").is_none());
                assert!(
                    payload["body"]
                        .as_str()
                        .unwrap()
                        .starts_with("please explain\n\n<!-- reviewfox:operation=")
                );
                serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]})
            };
            Ok(http::Response::builder()
                .status(200)
                .body(AsyncBody::from(response.to_string()))
                .unwrap())
        });
        let store = PublicationStore::new(directory.path().join("remote"));
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let result = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        assert!(matches!(result.state, PublicationState::Published(_)));
        let saved = store.load(&comparison(), &origin(), 1).unwrap().unwrap();
        assert_eq!(saved, result);
        assert_eq!(open.current().comments[0].body, "please explain");
        assert!(
            !crate::export::export_review(open.review(), open.contexts())
                .contains("reviewfox:operation")
        );
        let no_network =
            FakeHttpClient::create(|_| async { panic!("a confirmed create must not be repeated") });
        let reopened = PublicationService::new(no_network, store);
        assert_eq!(
            futures::executor::block_on(reopened.publish(
                open.review(),
                open.contexts(),
                &origin(),
                1,
                &connection
            ))
            .unwrap(),
            saved
        );
    }
}
