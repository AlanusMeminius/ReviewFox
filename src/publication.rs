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
        self.preparation_eligibility(comparison)?;
        let Self::GitLab { start_sha, .. } = self else {
            unreachable!()
        };
        if start_sha
            .as_deref()
            .is_none_or(|sha| sha.parse::<Oid>().is_err())
        {
            return Err("MR version information incomplete · publication unavailable");
        }
        Ok(())
    }
}

use crate::domain::{DraftComment, Review};
use crate::export::ReviewContexts;
use crate::gitlab_publication::{GitLabDiffPosition, GitLabPublication, RemoteNote, UpdateOutcome};
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
pub struct PublicationReceipt {
    pub discussion_id: String,
    pub note_id: u64,
    pub confirmed_body: String,
    pub remote_position: serde_json::Value,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub outdated: Option<bool>,
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
pub enum EditStatus {
    Pending,
    Sending { sent_body: String },
    Failed(String),
    Unknown { sent_body: String, message: String },
    Conflict { website_body: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEdit {
    pub body: String,
    pub status: EditStatus,
}

pub struct RemoteRead {
    pub note: Option<RemoteNote>,
    pub confirmed_body: String,
}

pub struct RefreshOutcome {
    pub record: PublicationRecord,
    pub adopt_body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationRecord {
    pub operation_id: String,
    pub body: String,
    pub author_id: Option<u64>,
    pub position: Option<GitLabDiffPosition>,
    pub state: PublicationState,
    #[serde(default)]
    pub edit: Option<PendingEdit>,
    #[serde(default)]
    pub website_body: Option<String>,
    #[serde(default)]
    pub website_deleted: bool,
}
impl PublicationRecord {
    pub fn receipt(&self) -> Option<&PublicationReceipt> {
        match &self.state {
            PublicationState::Published(receipt) => Some(receipt),
            _ => None,
        }
    }
    fn observe_remote(&mut self, remote: &RemoteNote) {
        self.website_deleted = false;
        self.website_body = Some(remote.body.clone());
        if let PublicationState::Published(receipt) = &mut self.state {
            receipt.remote_position = remote.position.clone();
            receipt.resolved = remote.resolved;
            receipt.outdated = remote.outdated;
        }
    }
    pub fn status_label(&self) -> String {
        if self.website_deleted {
            return "Website deleted · local comment retained".into();
        }
        match self.edit.as_ref().map(|edit| &edit.status) {
            Some(EditStatus::Pending) => "Local changes pending".into(),
            Some(EditStatus::Sending { .. }) => "Updating GitLab…".into(),
            Some(EditStatus::Failed(message)) => format!("Update failed: {message}"),
            Some(EditStatus::Unknown { message, .. }) => {
                format!("Update result unknown: {message}")
            }
            Some(EditStatus::Conflict { .. }) => "Local and website bodies conflict".into(),
            None if self.website_body.as_ref().is_some_and(|website| {
                self.receipt()
                    .is_some_and(|receipt| website != &receipt.confirmed_body)
            }) =>
            {
                "Website changed · active editor preserved".into()
            }
            None => self.state.label(),
        }
    }
    pub fn visible_body(&self, raw: &str) -> String {
        raw.replace(&format!("\n\n{}", self.marker()), "")
            .replace(&self.marker(), "")
    }

    pub fn marker(&self) -> String {
        format!("<!-- reviewfox:operation={} -->", self.operation_id)
    }
    pub fn remote_body(&self) -> String {
        format!("{}\n\n{}", self.body, self.marker())
    }
}

#[derive(Clone, Debug)]
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
    /// Any remote counterpart or uncertain create protects the shared local draft.
    pub fn protected_comments(
        &self,
        comparison: &Comparison,
    ) -> Result<std::collections::HashSet<u64>> {
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Default::default());
            }
            Err(error) => return Err(error).context("Cannot inspect Publication storage"),
        };
        let mut protected = std::collections::HashSet::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let saved: StoredPublication = serde_json::from_slice(&std::fs::read(&path)?)
                .with_context(|| {
                    format!("Unreadable Publication preserved at {}", path.display())
                })?;
            if saved.comparison != *comparison {
                continue;
            }
            let record = self
                .load(comparison, &saved.origin, saved.comment_id)?
                .context("Publication disappeared while reading")?;
            if !matches!(record.state, PublicationState::Failed(_)) {
                protected.insert(saved.comment_id);
            }
        }
        Ok(protected)
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
    fn known(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
    ) -> Result<PublicationRecord> {
        let record = self
            .store
            .load(comparison, origin, id)?
            .context("Publication no longer exists")?;
        if record.receipt().is_none() {
            bail!("No confirmed remote identity is available");
        }
        Ok(record)
    }
    async fn authorize(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> Result<()> {
        if !connection.matches(origin) || connection.pat.trim().is_empty() {
            bail!("Reconnect this Review's GitLab host and token");
        }
        let author = self
            .adapter
            .author(origin, connection)
            .await
            .map_err(anyhow::Error::msg)?;
        if record.author_id != Some(author) {
            bail!("GitLab account changed; reconnect the original account");
        }
        Ok(())
    }
    /// Reads only the known counterpart. Local edits are coordinated after the read completes.
    pub async fn read(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        connection: &Connection,
    ) -> Result<RemoteRead> {
        let record = self.known(comparison, origin, id)?;
        self.authorize(origin, connection, &record).await?;
        let note = self
            .adapter
            .read_known(origin, connection, &record)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(RemoteRead {
            note,
            confirmed_body: record.receipt().unwrap().confirmed_body.clone(),
        })
    }

    pub fn reconcile(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        local_body: &str,
        editing: bool,
        remote: RemoteRead,
    ) -> Result<RefreshOutcome> {
        let mut record = self.known(comparison, origin, id)?;
        let mut adopt_body = None;
        if record.receipt().unwrap().confirmed_body != remote.confirmed_body {
            return Ok(RefreshOutcome { record, adopt_body });
        }
        match remote.note {
            None => record.website_deleted = true,
            Some(remote) => {
                record.observe_remote(&remote);
                let confirmed = record.receipt().unwrap().confirmed_body.clone();
                let busy = record.edit.as_ref().is_some_and(|edit| {
                    matches!(
                        edit.status,
                        EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                    )
                });
                if !busy {
                    if !editing && (local_body == confirmed || local_body == remote.body) {
                        if local_body != remote.body {
                            adopt_body = Some(remote.body.clone());
                        }
                        if let PublicationState::Published(receipt) = &mut record.state {
                            receipt.confirmed_body = remote.body.clone();
                        }
                        record.edit = None;
                    } else if local_body != confirmed {
                        record.edit = Some(PendingEdit {
                            body: local_body.to_owned(),
                            status: if remote.body != confirmed && local_body != remote.body {
                                EditStatus::Conflict {
                                    website_body: remote.body.clone(),
                                }
                            } else {
                                record
                                    .edit
                                    .as_ref()
                                    .filter(|edit| edit.body == local_body)
                                    .map(|edit| edit.status.clone())
                                    .filter(|status| matches!(status, EditStatus::Failed(_)))
                                    .unwrap_or(EditStatus::Pending)
                            },
                        });
                    }
                }
            }
        }
        self.store.save(comparison, origin, id, &record)?;
        Ok(RefreshOutcome { record, adopt_body })
    }

    pub fn queue_edit(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        body: &str,
    ) -> Result<PublicationRecord> {
        let mut record = self.known(comparison, origin, id)?;
        let status = record
            .edit
            .as_ref()
            .map(|edit| edit.status.clone())
            .filter(|status| {
                matches!(
                    status,
                    EditStatus::Sending { .. }
                        | EditStatus::Unknown { .. }
                        | EditStatus::Conflict { .. }
                )
            })
            .unwrap_or(EditStatus::Pending);
        if body == record.receipt().unwrap().confirmed_body && matches!(status, EditStatus::Pending)
        {
            record.edit = None;
        } else {
            record.edit = Some(PendingEdit {
                body: body.into(),
                status,
            });
        }
        self.store.save(comparison, origin, id, &record)?;
        Ok(record)
    }
    pub fn adopt_website(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
    ) -> Result<RefreshOutcome> {
        let mut record = self.known(comparison, origin, id)?;
        if record.website_deleted
            || record.edit.as_ref().is_some_and(|edit| {
                matches!(
                    edit.status,
                    EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                )
            })
        {
            bail!("Cannot adopt while the remote outcome is uncertain");
        }
        let body = record
            .website_body
            .clone()
            .context("No website body available")?;
        if let PublicationState::Published(receipt) = &mut record.state {
            receipt.confirmed_body = body.clone();
        }
        record.edit = None;
        self.store.save(comparison, origin, id, &record)?;
        Ok(RefreshOutcome {
            record,
            adopt_body: Some(body),
        })
    }
    /// Records an interrupted send conservatively, without initiating any network write.
    pub fn restore_interrupted(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
    ) -> Result<PublicationRecord> {
        let mut record = self.known(comparison, origin, id)?;
        if let Some(edit) = &mut record.edit {
            if let EditStatus::Sending { sent_body } = &edit.status {
                edit.status = EditStatus::Unknown {
                    sent_body: sent_body.clone(),
                    message: "Previous update confirmation unavailable".into(),
                };
                self.store.save(comparison, origin, id, &record)?;
            }
        }
        Ok(record)
    }
    pub async fn update(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        connection: &Connection,
        overwrite: bool,
    ) -> Result<PublicationRecord> {
        let mut record = self.known(comparison, origin, id)?;
        let Some(edit) = record.edit.as_ref() else {
            return Ok(record);
        };
        if record.website_deleted
            || matches!(
                edit.status,
                EditStatus::Sending { .. } | EditStatus::Unknown { .. }
            )
        {
            return Ok(record);
        }
        let sent_body = edit.body.clone();
        let expected_website = match &edit.status {
            EditStatus::Conflict { website_body } => Some(website_body.clone()),
            _ => None,
        };
        record.edit.as_mut().unwrap().status = EditStatus::Sending {
            sent_body: sent_body.clone(),
        };
        self.store.save(comparison, origin, id, &record)?;
        let remote = self
            .read(comparison, origin, id, connection)
            .await
            .map(|snapshot| snapshot.note);
        let outcome = match remote {
            Err(error) => UpdateOutcome::Failed(error.to_string()),
            Ok(None) => {
                let mut current = self.known(comparison, origin, id)?;
                current.website_deleted = true;
                if let Some(edit) = &mut current.edit {
                    edit.status = EditStatus::Pending;
                }
                self.store.save(comparison, origin, id, &current)?;
                return Ok(current);
            }
            Ok(Some(remote)) => {
                let confirmed = record.receipt().unwrap().confirmed_body.clone();
                if remote.body != confirmed
                    && remote.body != sent_body
                    && (!overwrite || expected_website.as_ref() != Some(&remote.body))
                {
                    let mut current = self.known(comparison, origin, id)?;
                    current.observe_remote(&remote);
                    if let Some(edit) = &mut current.edit {
                        edit.status = EditStatus::Conflict {
                            website_body: remote.body,
                        };
                    }
                    self.store.save(comparison, origin, id, &current)?;
                    return Ok(current);
                }
                if remote.body == sent_body {
                    UpdateOutcome::Confirmed(remote)
                } else {
                    record.body = sent_body.clone();
                    self.adapter.update_known(origin, connection, &record).await
                }
            }
        };
        // Re-read after awaiting: a newer explicit save may have queued another body.
        let mut current = self.known(comparison, origin, id)?;
        match outcome {
            UpdateOutcome::Confirmed(remote) => {
                current.observe_remote(&remote);
                if let PublicationState::Published(receipt) = &mut current.state {
                    receipt.confirmed_body = remote.body.clone();
                }
                if current
                    .edit
                    .as_ref()
                    .is_none_or(|edit| edit.body == remote.body)
                {
                    current.edit = None;
                } else if let Some(edit) = &mut current.edit {
                    edit.status = EditStatus::Pending;
                }
            }
            UpdateOutcome::Failed(message) => {
                if let Some(edit) = &mut current.edit {
                    edit.status = EditStatus::Failed(message);
                }
            }
            UpdateOutcome::Unknown(message) => {
                if let Some(edit) = &mut current.edit {
                    edit.status = EditStatus::Unknown { sent_body, message };
                }
            }
        }
        self.store.save(comparison, origin, id, &current)?;
        Ok(current)
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
            edit: None,
            website_body: None,
            website_deleted: false,
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

    fn published(directory: &std::path::Path) -> (OpenReview, PublicationStore) {
        let open = drafted(directory, Side::Postimage, 2);
        let store = PublicationStore::new(directory.join("remote"));
        let client = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            Ok(response(
                serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
            ))
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        assert!(matches!(
            futures::executor::block_on(service.publish(
                open.review(),
                open.contexts(),
                &origin(),
                1,
                &connection
            ))
            .unwrap()
            .state,
            PublicationState::Published(_)
        ));
        (open.with_publication_store(store.clone()), store)
    }
    fn website_client(body: &str) -> Arc<dyn HttpClient> {
        let body = body.to_owned();
        FakeHttpClient::create(move |request| {
            let body = body.clone();
            async move {
                assert_eq!(request.method(), http::Method::GET);
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":body,"author":{"id":42},"position":{"new_line":9},"resolved":false}]}),
                ))
            }
        })
    }
    #[test]
    fn conflicting_saved_bodies_require_explicit_adoption_and_refresh_preserves_active_editor() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let service = PublicationService::new(website_client("website body"), store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        open.begin_edit(1);
        let remote =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        let outcome = service
            .reconcile(&comparison(), &origin(), 1, "please explain", true, remote)
            .unwrap();
        assert!(outcome.adopt_body.is_none());
        assert_eq!(open.current().comments[0].body, "please explain");
        assert!(open.dock().is_some());
        open.commit("local body");
        service
            .queue_edit(&comparison(), &origin(), 1, "local body")
            .unwrap();
        let remote =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        let outcome = service
            .reconcile(&comparison(), &origin(), 1, "local body", false, remote)
            .unwrap();
        assert!(
            matches!(outcome.record.edit.unwrap().status,EditStatus::Conflict {website_body} if website_body=="website body")
        );
        assert_eq!(open.current().comments[0].body, "local body");
        let adopted = service.adopt_website(&comparison(), &origin(), 1).unwrap();
        open.adopt_body(1, &adopted.adopt_body.unwrap());
        assert_eq!(open.current().comments[0].body, "website body");
        assert!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .edit
                .is_none()
        );
    }
    #[test]
    fn later_saved_body_survives_earlier_update_response_and_stale_refresh() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let original = store.load(&comparison(), &origin(), 1).unwrap().unwrap();
        let marker = original.marker();
        let (release, wait) = futures::channel::oneshot::channel::<()>();
        let waiting = Arc::new(std::sync::Mutex::new(Some(wait)));
        let client = FakeHttpClient::create(move |mut request| {
            let waiting = waiting.clone();
            let marker = marker.clone();
            async move {
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.method() == http::Method::GET {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42},"position":{"new_line":2}}]}),
                    ));
                }
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(payload["body"], format!("body B\n\n{marker}"));
                let wait = waiting.lock().unwrap().take().unwrap();
                wait.await.unwrap();
                Ok(response(
                    serde_json::json!({"id":51,"body":payload["body"],"author":{"id":42},"position":{"new_line":2}}),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let stale =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        open.begin_edit(1);
        open.commit("body B");
        service
            .queue_edit(&comparison(), &origin(), 1, "body B")
            .unwrap();
        let cmp = comparison();
        let target = origin();
        let mut update = Box::pin(service.update(&cmp, &target, 1, &connection, false));
        assert!(update.as_mut().now_or_never().is_none());
        open.begin_edit(1);
        open.commit("body C");
        service
            .queue_edit(&comparison(), &origin(), 1, "body C")
            .unwrap();
        release.send(()).unwrap();
        let result = futures::executor::block_on(update).unwrap();
        assert_eq!(result.receipt().unwrap().confirmed_body, "body B");
        assert!(
            matches!(result.edit,Some(PendingEdit {body,status:EditStatus::Pending}) if body=="body C")
        );
        assert_eq!(open.current().comments[0].body, "body C");
        let outcome = service
            .reconcile(&comparison(), &origin(), 1, "body C", false, stale)
            .unwrap();
        assert!(outcome.adopt_body.is_none());
        assert_eq!(outcome.record.receipt().unwrap().confirmed_body, "body B");
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        );
        assert_eq!(reopened.current().comments[0].body, "body C");
    }
    #[test]
    fn failed_and_unknown_updates_preserve_prior_receipt_and_never_retry_on_read() {
        for status in [403, 0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let client = FakeHttpClient::create(move |request| async move {
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.method() == http::Method::GET {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42},"position":{"new_line":2}}]}),
                    ));
                }
                if status == 0 {
                    return Err(anyhow::anyhow!("private token detail"));
                }
                Ok(http::Response::builder()
                    .status(status)
                    .body(AsyncBody::empty())
                    .unwrap())
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            open.begin_edit(1);
            open.commit("body B");
            service
                .queue_edit(&comparison(), &origin(), 1, "body B")
                .unwrap();
            let record = futures::executor::block_on(service.update(
                &comparison(),
                &origin(),
                1,
                &connection,
                false,
            ))
            .unwrap();
            assert_eq!(record.receipt().unwrap().confirmed_body, "please explain");
            if status == 0 {
                assert!(
                    matches!(record.edit.as_ref().unwrap().status,EditStatus::Unknown {ref sent_body,..} if sent_body=="body B")
                );
            } else {
                assert!(matches!(
                    record.edit.as_ref().unwrap().status,
                    EditStatus::Failed(_)
                ));
            }
            assert!(!record.status_label().contains("private"));
            let reopened = PublicationService::new(website_client("please explain"), store.clone());
            let remote = futures::executor::block_on(reopened.read(
                &comparison(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
            let outcome = reopened
                .reconcile(&comparison(), &origin(), 1, "body B", false, remote)
                .unwrap();
            assert_eq!(
                outcome.record.edit, record.edit,
                "read-only reopen retains failure or uncertainty"
            );
            assert_eq!(open.current().comments[0].body, "body B");
        }
    }
    #[test]
    fn only_verified_remote_absence_marks_website_deleted() {
        for forbidden in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let (open, store) = published(directory.path());
            let client = FakeHttpClient::create(move |request| async move {
                assert_eq!(request.method(), http::Method::GET);
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.uri().path().ends_with("/discussions/discussion-1") {
                    return Ok(http::Response::builder()
                        .status(404)
                        .body(AsyncBody::empty())
                        .unwrap());
                }
                if forbidden {
                    return Ok(http::Response::builder()
                        .status(403)
                        .body(AsyncBody::empty())
                        .unwrap());
                }
                Ok(response(serde_json::json!([])))
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            let result =
                futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection));
            if forbidden {
                assert!(result.is_err());
                assert!(
                    !store
                        .load(&comparison(), &origin(), 1)
                        .unwrap()
                        .unwrap()
                        .website_deleted
                );
            } else {
                let outcome = service
                    .reconcile(
                        &comparison(),
                        &origin(),
                        1,
                        &open.current().comments[0].body,
                        false,
                        result.unwrap(),
                    )
                    .unwrap();
                assert!(outcome.record.website_deleted);
                assert!(outcome.adopt_body.is_none());
            }
            assert_eq!(open.current().comments[0].body, "please explain");
        }
    }

    #[test]
    fn overwrite_rechecks_the_body_shown_in_conflict_before_updating() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        open.begin_edit(1);
        open.commit("local C");
        let first = PublicationService::new(website_client("website B"), store.clone());
        first
            .queue_edit(&comparison(), &origin(), 1, "local C")
            .unwrap();
        let conflict = futures::executor::block_on(first.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        assert!(
            matches!(conflict.edit.as_ref().unwrap().status,EditStatus::Conflict {ref website_body} if website_body=="website B")
        );
        assert_eq!(
            conflict.receipt().unwrap().remote_position,
            serde_json::json!({"new_line":9})
        );
        assert_eq!(conflict.receipt().unwrap().resolved, Some(false));
        let second = PublicationService::new(website_client("website D"), store.clone());
        let changed = futures::executor::block_on(second.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            true,
        ))
        .unwrap();
        assert!(
            matches!(changed.edit.as_ref().unwrap().status,EditStatus::Conflict {ref website_body} if website_body=="website D")
        );
        let client = FakeHttpClient::create(|mut request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.method() == http::Method::GET {
                return Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"website D","author":{"id":42},"position":{"new_line":9}}]}),
                ));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                payload["body"]
                    .as_str()
                    .unwrap()
                    .starts_with("local C\n\n<!-- reviewfox:operation=")
            );
            Ok(response(
                serde_json::json!({"id":51,"body":payload["body"],"author":{"id":42},"position":{"new_line":9}}),
            ))
        });
        let confirmed = PublicationService::new(client, store);
        let record = futures::executor::block_on(confirmed.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            true,
        ))
        .unwrap();
        assert_eq!(record.receipt().unwrap().confirmed_body, "local C");
        assert!(record.edit.is_none());
    }

    #[test]
    fn saving_a_published_body_updates_the_same_native_note_and_preserves_marker() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let old = store.load(&comparison(), &origin(), 1).unwrap().unwrap();
        let marker = old.marker();
        open.begin_edit(1);
        open.commit("new local body");
        let client = FakeHttpClient::create(move |mut request| {
            let marker = marker.clone();
            async move {
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.method() == http::Method::GET {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":format!("please explain\n\n{marker}"),"author":{"id":42},"position":{"new_line":2}}]}),
                    ));
                }
                assert_eq!(request.method(), http::Method::PUT);
                assert!(
                    request
                        .uri()
                        .path()
                        .ends_with("/discussions/discussion-1/notes/51")
                );
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(payload["body"], format!("new local body\n\n{marker}"));
                assert!(payload.get("position").is_none());
                Ok(response(
                    serde_json::json!({"id":51,"body":payload["body"],"author":{"id":42},"position":{"new_line":2}}),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        service
            .queue_edit(&comparison(), &origin(), 1, "new local body")
            .unwrap();
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let record = futures::executor::block_on(service.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        assert_eq!(record.receipt().unwrap().note_id, 51);
        assert_eq!(record.receipt().unwrap().confirmed_body, "new local body");
        assert!(record.edit.is_none());
        assert_eq!(open.current().comments[0].body, "new local body");
    }

    #[test]
    fn refresh_adopts_website_only_edits_without_changing_anchor_or_export_context() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let before = open.current().comments[0].anchor.clone();
        let context = open.contexts().clone();
        let client = FakeHttpClient::create(|request| async move {
            assert_eq!(request.method(), http::Method::GET);
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            assert!(request.uri().path().ends_with("/discussions/discussion-1"));
            Ok(response(
                serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"website revision","author":{"id":42},"position":{"new_line":8},"resolved":true}]}),
            ))
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let remote =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        let result = service
            .reconcile(
                &comparison(),
                &origin(),
                1,
                &open.current().comments[0].body,
                false,
                remote,
            )
            .unwrap();
        open.adopt_body(1, &result.adopt_body.unwrap());
        assert_eq!(open.current().comments[0].body, "website revision");
        assert_eq!(open.current().comments[0].anchor, before);
        assert_eq!(open.contexts(), &context);
        let PublicationState::Published(receipt) = result.record.state else {
            panic!("known correspondence retained")
        };
        assert_eq!(receipt.confirmed_body, "website revision");
        assert_eq!(receipt.resolved, Some(true));
        assert_eq!(receipt.remote_position, serde_json::json!({"new_line":8}));
        assert_eq!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .edit,
            None
        );
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
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        open = open.with_publication_store(store.clone());
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
        open.show_origin(comparison(), "a.rs", ReviewOrigin::Local);
        assert_eq!(
            open.delete(1).comments.len(),
            1,
            "an uncertain create on another origin must keep shared work accessible"
        );
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
        let reopened = PublicationService::new(no_network, store.clone());
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
        open = open.with_publication_store(store);
        let mut other = origin();
        if let ReviewOrigin::GitLab { iid, .. } = &mut other {
            *iid = 11;
        }
        open.show_origin(comparison(), "a.rs", other);
        assert_eq!(
            open.delete(1).comments.len(),
            1,
            "a receipt on another target must stay reachable"
        );
    }
}
