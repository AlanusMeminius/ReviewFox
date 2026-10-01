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
    pub fn web_url(&self, note_id: Option<u64>) -> Option<String> {
        let Self::GitLab {
            base_url,
            project,
            iid,
            ..
        } = self
        else {
            return None;
        };
        let project = project
            .split('/')
            .map(|segment| {
                url::form_urlencoded::byte_serialize(segment.as_bytes()).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("/");
        let mut url = format!(
            "{}/{project}/-/merge_requests/{iid}",
            base_url.trim_end_matches('/')
        );
        if let Some(id) = note_id {
            url.push_str(&format!("#note_{id}"));
        }
        Some(url)
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
use crate::gitlab_publication::{
    DeleteOutcome, GitLabDiffPosition, GitLabPublication, RemoteNote, UpdateOutcome,
};
use anyhow::{Context, Result, bail};
use gpui_http_client::HttpClient;
use std::{path::PathBuf, sync::Arc};

/// A target-scoped operation key. Local Review identity remains the Comparison.
#[derive(Clone, Debug)]
pub struct PublicationKey {
    pub comparison: Comparison,
    pub origin: ReviewOrigin,
    pub comment_id: u64,
}
impl PublicationKey {
    pub fn new(comparison: Comparison, origin: ReviewOrigin, comment_id: u64) -> Self {
        Self {
            comparison,
            origin,
            comment_id,
        }
    }
    pub fn matches_review(&self, comparison: &Comparison, origin: &ReviewOrigin) -> bool {
        self.comparison == *comparison && self.origin.same_target(origin)
    }
}
impl PartialEq for PublicationKey {
    fn eq(&self, other: &Self) -> bool {
        self.comment_id == other.comment_id && self.matches_review(&other.comparison, &other.origin)
    }
}
impl Eq for PublicationKey {}
impl std::hash::Hash for PublicationKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(
            &(
                &self.comparison,
                self.origin.target_identity(),
                self.comment_id,
            ),
            state,
        );
    }
}

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
impl PublicationReceipt {
    /// Server placement, resolution and outdated capability are independent of
    /// local body synchronization and the Review's captured Anchor.
    pub fn placement_label(&self) -> String {
        let position = &self.remote_position;
        let path = position
            .get("new_path")
            .and_then(|value| value.as_str())
            .filter(|path| !path.is_empty())
            .or_else(|| position.get("old_path").and_then(|value| value.as_str()));
        let location = path
            .map(|path| {
                let line = position
                    .get("new_line")
                    .and_then(|value| value.as_u64())
                    .map(|line| format!(" +{line}"))
                    .or_else(|| {
                        position
                            .get("old_line")
                            .and_then(|value| value.as_u64())
                            .map(|line| format!(" −{line}"))
                    })
                    .unwrap_or_default();
                format!("GitLab attachment: {path}{line}")
            })
            .unwrap_or_else(|| "GitLab attachment unavailable".into());
        let outdated = match self.outdated {
            Some(true) => "Outdated on GitLab",
            Some(false) => "Current on GitLab",
            None => "Outdated status unavailable",
        };
        let resolved = match self.resolved {
            Some(true) => "Resolved on GitLab",
            Some(false) => "Open on GitLab",
            None => "Resolution status unavailable",
        };
        format!("{location} · {outdated} · {resolved}")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicationState {
    Published(PublicationReceipt),
    Failed(String),
    Unsupported(String),
    Unknown(String),
}
impl PublicationState {
    pub fn label(&self) -> String {
        match self {
            Self::Published(_) => "Published on GitLab".into(),
            Self::Failed(message) => format!("Publish failed: {message}"),
            Self::Unsupported(message) => format!("Publication unavailable: {message}"),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteStatus {
    Pending,
    Sending,
    Failed(String),
    Unknown(String),
    Conflict { website_body: String },
    Confirmed,
}

pub struct RemoteRead {
    pub operation_id: String,
    pub note: Option<RemoteNote>,
    pub confirmed_body: String,
}

pub enum CheckedPublication {
    Create {
        operation_id: String,
        candidates: Vec<PublicationReceipt>,
    },
    Known(RemoteRead),
}

pub struct RefreshOutcome {
    pub record: PublicationRecord,
    pub adopt_body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainAttempt {
    pub operation_id: String,
    pub sent_body: String,
    pub author_id: Option<u64>,
    pub position: Option<GitLabDiffPosition>,
    pub candidates: Vec<PublicationReceipt>,
}

/// A verified missing counterpart resolves write uncertainty but retains sent evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsentUpdate {
    pub receipt: PublicationReceipt,
    pub sent_body: String,
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
    #[serde(default)]
    pub deletion: Option<DeleteStatus>,
    #[serde(default)]
    pub create_queued: bool,
    #[serde(default)]
    pub deletion_body: Option<String>,
    #[serde(default)]
    pub recovery_candidates: Vec<PublicationReceipt>,
    #[serde(default)]
    pub prior_attempts: Vec<UncertainAttempt>,
    #[serde(default)]
    pub absent_updates: Vec<AbsentUpdate>,
}
impl PublicationRecord {
    fn recreate_requested(&self) -> bool {
        self.website_deleted || self.deletion == Some(DeleteStatus::Confirmed)
    }
    fn fresh_create(comment: &DraftComment, previous: Option<&Self>, queued: bool) -> Self {
        Self {
            operation_id: previous
                .filter(|record| !record.recreate_requested())
                .map(|record| record.operation_id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            body: comment.body.clone(),
            author_id: previous.and_then(|record| record.author_id),
            absent_updates: previous
                .map(|record| record.absent_updates.clone())
                .unwrap_or_default(),
            prior_attempts: previous
                .map(|record| record.prior_attempts.clone())
                .unwrap_or_default(),
            position: None,
            state: PublicationState::Unknown(
                if queued {
                    "Create queued; confirmation unavailable"
                } else {
                    "Confirmation unavailable; check GitLab before publishing again"
                }
                .into(),
            ),
            edit: None,
            website_body: None,
            website_deleted: false,
            deletion: None,
            create_queued: queued,
            deletion_body: None,
            recovery_candidates: Vec::new(),
        }
    }
    /// Shared eligibility for the toolbar count and durable batch reservation.
    pub fn batch_eligible(record: Option<&Self>) -> bool {
        record.is_none_or(|record| {
            matches!(record.state, PublicationState::Failed(_))
                && record.deletion != Some(DeleteStatus::Confirmed)
        })
    }
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
        if self.website_deleted
            && !matches!(
                self.deletion,
                Some(DeleteStatus::Sending | DeleteStatus::Unknown(_) | DeleteStatus::Confirmed)
            )
        {
            return "Website deleted · local comment retained".into();
        }
        if let Some(deletion) = &self.deletion {
            let label = match deletion {
                DeleteStatus::Pending if !self.website_deleted => {
                    Some("Delete pending · local comment retained".into())
                }
                DeleteStatus::Sending => Some("Deleting from GitLab…".into()),
                DeleteStatus::Failed(message) => Some(format!("Delete failed: {message}")),
                DeleteStatus::Unknown(message) => Some(format!("Delete result unknown: {message}")),
                DeleteStatus::Conflict { .. } => {
                    Some("Website changed · confirm deletion again".into())
                }
                DeleteStatus::Confirmed => Some("Deleted from GitLab · local work retained".into()),
                DeleteStatus::Pending => None,
            };
            if let Some(label) = label {
                return label;
            }
        }
        if self.website_deleted {
            return "Website deleted · local comment retained".into();
        }
        if self.receipt().is_none() {
            return self.state.label();
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
        let mut body = raw.to_owned();
        for operation in std::iter::once(&self.operation_id).chain(
            self.prior_attempts
                .iter()
                .map(|attempt| &attempt.operation_id),
        ) {
            let marker = format!("<!-- reviewfox:operation={operation} -->");
            body = body
                .replace(&format!("\n\n{marker}"), "")
                .replace(&marker, "");
        }
        body
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
    pub fn load_key(&self, key: &PublicationKey) -> Result<Option<PublicationRecord>> {
        self.load(&key.comparison, &key.origin, key.comment_id)
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
            if !matches!(
                record.state,
                PublicationState::Failed(_) | PublicationState::Unsupported(_)
            ) && record.deletion != Some(DeleteStatus::Confirmed)
            {
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

/// An explicitly reserved batch advances only one item per UI callback.
/// Dropping it makes every unsent reservation manually retryable without any HTTP write.
pub struct PublicationBatch {
    review: Review,
    contexts: ReviewContexts,
    origin: ReviewOrigin,
    store: PublicationStore,
    pending: std::collections::VecDeque<(u64, String)>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Clone)]
pub struct BatchCancellation(Arc<std::sync::atomic::AtomicBool>);
impl BatchCancellation {
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
impl PublicationBatch {
    pub fn cancellation(&self) -> BatchCancellation {
        BatchCancellation(self.cancelled.clone())
    }
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.pending.iter().map(|item| item.0)
    }
    pub async fn publish_next(
        &mut self,
        service: &PublicationService,
        connection: &Connection,
    ) -> Result<Option<(u64, PublicationRecord)>> {
        if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(None);
        }
        let Some((id, _)) = self.pending.pop_front() else {
            return Ok(None);
        };
        let record = service
            .publish_with_cancellation(
                &self.review,
                &self.contexts,
                &self.origin,
                id,
                connection,
                Some(&self.cancelled),
            )
            .await?;
        Ok(Some((id, record)))
    }
}
impl Drop for PublicationBatch {
    fn drop(&mut self) {
        for (id, operation) in &self.pending {
            let result = (|| -> Result<()> {
                let Some(mut record) =
                    self.store
                        .load(&self.review.comparison, &self.origin, *id)?
                else {
                    return Ok(());
                };
                if record.operation_id == *operation && record.create_queued {
                    record.create_queued = false;
                    record.state = PublicationState::Failed(
                        "Batch stopped before sending; retry explicitly".into(),
                    );
                    if record.deletion == Some(DeleteStatus::Pending) {
                        record.deletion = Some(DeleteStatus::Confirmed);
                    }
                    self.store
                        .save(&self.review.comparison, &self.origin, *id, &record)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                log::error!("Could not stop queued publication: {error:#}");
            }
        }
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
    pub fn draft_ids(&self, review: &Review, origin: &ReviewOrigin) -> Result<Vec<u64>> {
        origin
            .preparation_eligibility(&review.comparison)
            .map_err(anyhow::Error::msg)?;
        let mut ids = Vec::new();
        for comment in &review.comments {
            let record = self.store.load(&review.comparison, origin, comment.id)?;
            if PublicationRecord::batch_eligible(record.as_ref()) {
                ids.push(comment.id);
            }
        }
        Ok(ids)
    }
    pub fn reserve_all(
        &self,
        review: &Review,
        contexts: &ReviewContexts,
        origin: &ReviewOrigin,
    ) -> Result<PublicationBatch> {
        let ids = self.draft_ids(review, origin)?;
        let mut batch = PublicationBatch {
            review: review.clone(),
            contexts: contexts.clone(),
            origin: origin.clone(),
            store: self.store.clone(),
            pending: Default::default(),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        for id in ids {
            let record = self.queue_create(review, origin, id)?;
            batch.pending.push_back((id, record.operation_id));
        }
        Ok(batch)
    }
    fn save_create(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        record: &mut PublicationRecord,
    ) -> Result<()> {
        if let Some(current) = self.store.load(comparison, origin, id)? {
            if current.operation_id == record.operation_id {
                record.deletion = current.deletion;
                record.deletion_body = current.deletion_body;
                record.edit = current.edit;
            }
        }
        if matches!(
            record.state,
            PublicationState::Failed(_) | PublicationState::Unsupported(_)
        ) && record.deletion == Some(DeleteStatus::Pending)
        {
            record.deletion = Some(DeleteStatus::Confirmed);
        }
        self.store.save(comparison, origin, id, record)
    }
    /// Durably reserves the explicit create before a UI background task can be scheduled.
    pub fn queue_create(
        &self,
        review: &Review,
        origin: &ReviewOrigin,
        id: u64,
    ) -> Result<PublicationRecord> {
        origin
            .preparation_eligibility(&review.comparison)
            .map_err(anyhow::Error::msg)?;
        let previous = self.store.load(&review.comparison, origin, id)?;
        if previous.as_ref().is_some_and(|record| {
            matches!(record.state, PublicationState::Unknown(_))
                || matches!(
                    record.deletion,
                    Some(DeleteStatus::Sending | DeleteStatus::Unknown(_))
                )
                || record.edit.as_ref().is_some_and(|edit| {
                    matches!(
                        edit.status,
                        EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                    )
                })
        }) {
            bail!("Previous write remains uncertain; check GitLab first");
        }
        let recreate = previous
            .as_ref()
            .is_some_and(PublicationRecord::recreate_requested);
        if previous
            .as_ref()
            .is_some_and(|record| record.receipt().is_some() && !recreate)
        {
            bail!("Publication already exists");
        }
        let comment = review
            .comments
            .iter()
            .find(|comment| comment.id == id)
            .context("DraftComment no longer exists")?;
        let record = PublicationRecord::fresh_create(comment, previous.as_ref(), true);
        self.store.save(&review.comparison, origin, id, &record)?;
        Ok(record)
    }

    /// A new attempt requires explicit acknowledgement that the old one may exist.
    pub fn queue_republish(
        &self,
        review: &Review,
        origin: &ReviewOrigin,
        id: u64,
        duplicate_risk_confirmed: bool,
    ) -> Result<PublicationRecord> {
        if !duplicate_risk_confirmed {
            bail!("Republishing may create a duplicate; explicit confirmation required");
        }
        let mut record = self
            .store
            .load(&review.comparison, origin, id)?
            .context("No uncertain attempt is available")?;
        if record.receipt().is_some() || !matches!(record.state, PublicationState::Unknown(_)) {
            bail!("This create is not uncertain");
        }
        if record.deletion.is_some() {
            bail!(
                "Deletion intent remains pending; check the original attempt before republishing"
            );
        }
        record.prior_attempts.push(UncertainAttempt {
            operation_id: record.operation_id.clone(),
            sent_body: record.body.clone(),
            author_id: record.author_id,
            position: record.position.clone(),
            candidates: record.recovery_candidates.clone(),
        });
        record.operation_id = uuid::Uuid::new_v4().to_string();
        record.state =
            PublicationState::Failed("Explicit new attempt approved despite duplicate risk".into());
        record.create_queued = false;
        self.store.save(&review.comparison, origin, id, &record)?;
        self.queue_create(review, origin, id)
    }

    /// Cancel only an unsent deletion intent; uncertainty and recovery evidence remain.
    pub fn cancel_pending_delete(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
    ) -> Result<PublicationRecord> {
        let mut record = self
            .store
            .load(comparison, origin, id)?
            .context("Publication no longer exists")?;
        if record.deletion != Some(DeleteStatus::Pending) {
            bail!("Only a deletion that has not been sent can be cancelled");
        }
        record.deletion = None;
        record.deletion_body = None;
        self.store.save(comparison, origin, id, &record)?;
        Ok(record)
    }

    /// Preserve the draft and receipt until the remote deletion is confirmed.
    pub fn queue_delete(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        local_body: &str,
    ) -> Result<PublicationRecord> {
        let mut record = self
            .store
            .load(comparison, origin, id)?
            .context("Publication no longer exists")?;
        if record.deletion.is_none() || matches!(record.deletion, Some(DeleteStatus::Failed(_))) {
            record.deletion = Some(
                if matches!(
                    record.state,
                    PublicationState::Failed(_) | PublicationState::Unsupported(_)
                ) {
                    DeleteStatus::Confirmed
                } else {
                    DeleteStatus::Pending
                },
            );
            record.deletion_body = Some(local_body.into());
        }
        self.store.save(comparison, origin, id, &record)?;
        Ok(record)
    }

    pub async fn delete(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        connection: &Connection,
        confirm_changed_body: bool,
    ) -> Result<PublicationRecord> {
        let mut record = self.known(comparison, origin, id)?;
        if !matches!(
            record.deletion,
            Some(DeleteStatus::Pending | DeleteStatus::Failed(_) | DeleteStatus::Conflict { .. })
        ) || record.edit.as_ref().is_some_and(|edit| {
            matches!(
                edit.status,
                EditStatus::Sending { .. } | EditStatus::Unknown { .. }
            )
        }) {
            return Ok(record);
        }
        let expected = match &record.deletion {
            Some(DeleteStatus::Conflict { website_body }) => Some(website_body.clone()),
            _ => None,
        };
        record.deletion = Some(DeleteStatus::Sending);
        self.store.save(comparison, origin, id, &record)?;
        let outcome = match self.read(comparison, origin, id, connection).await {
            Err(error) => DeleteOutcome::Failed(error.to_string()),
            Ok(RemoteRead { note: None, .. }) => {
                let mut current = self.known(comparison, origin, id)?;
                current.website_deleted = true;
                current.deletion = Some(DeleteStatus::Pending);
                self.store.save(comparison, origin, id, &current)?;
                return Ok(current);
            }
            Ok(RemoteRead {
                note: Some(remote), ..
            }) => {
                if remote.body != record.receipt().unwrap().confirmed_body
                    && (!confirm_changed_body || expected.as_ref() != Some(&remote.body))
                {
                    let mut current = self.known(comparison, origin, id)?;
                    current.observe_remote(&remote);
                    current.deletion = Some(DeleteStatus::Conflict {
                        website_body: remote.body,
                    });
                    self.store.save(comparison, origin, id, &current)?;
                    return Ok(current);
                }
                self.adapter.delete_known(origin, connection, &record).await
            }
        };
        let mut current = self.known(comparison, origin, id)?;
        current.deletion = Some(match outcome {
            DeleteOutcome::Confirmed => DeleteStatus::Confirmed,
            DeleteOutcome::Failed(message) => DeleteStatus::Failed(message),
            DeleteOutcome::Unknown(message) => DeleteStatus::Unknown(message),
        });
        if current.deletion == Some(DeleteStatus::Confirmed) {
            current.website_deleted = true;
        }
        self.store.save(comparison, origin, id, &current)?;
        Ok(current)
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
            operation_id: record.operation_id.clone(),
            note,
            confirmed_body: record.receipt().unwrap().confirmed_body.clone(),
        })
    }

    /// This operation only reads; explicit reconciliation never initiates another write.
    pub async fn check_again(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        connection: &Connection,
    ) -> Result<CheckedPublication> {
        let record = self
            .store
            .load(comparison, origin, id)?
            .context("No publication intent is available")?;
        if record.receipt().is_some() {
            return self
                .read(comparison, origin, id, connection)
                .await
                .map(CheckedPublication::Known);
        }
        if !matches!(record.state, PublicationState::Unknown(_)) {
            bail!("This creation is not awaiting confirmation");
        }
        self.authorize(origin, connection, &record).await?;
        let candidates = self
            .adapter
            .find_marked(origin, connection, &record)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(CheckedPublication::Create {
            operation_id: record.operation_id,
            candidates,
        })
    }
    pub fn reconcile_check(
        &self,
        comparison: &Comparison,
        origin: &ReviewOrigin,
        id: u64,
        local_body: &str,
        editing: bool,
        checked: CheckedPublication,
    ) -> Result<RefreshOutcome> {
        match checked {
            CheckedPublication::Known(remote) => {
                let mut record = self.known(comparison, origin, id)?;
                if record.operation_id != remote.operation_id
                    || record.receipt().unwrap().confirmed_body != remote.confirmed_body
                {
                    return Ok(RefreshOutcome {
                        record,
                        adopt_body: None,
                    });
                }
                match &remote.note {
                    None => {
                        if matches!(
                            record.deletion,
                            Some(DeleteStatus::Unknown(_) | DeleteStatus::Sending)
                        ) {
                            record.deletion = Some(DeleteStatus::Confirmed);
                        }
                        if let Some(sent_body) =
                            record.edit.as_ref().and_then(|edit| match &edit.status {
                                EditStatus::Unknown { sent_body, .. }
                                | EditStatus::Sending { sent_body } => Some(sent_body.clone()),
                                _ => None,
                            })
                        {
                            record.absent_updates.push(AbsentUpdate {
                                receipt: record.receipt().unwrap().clone(),
                                sent_body,
                            });
                            record.edit.as_mut().unwrap().status = EditStatus::Failed("Website comment no longer exists; publish explicitly to create another".into());
                        }
                        record.website_deleted = true;
                    }
                    Some(note) => {
                        if matches!(
                            record.deletion,
                            Some(DeleteStatus::Unknown(_) | DeleteStatus::Sending)
                        ) {
                            record.deletion =
                                Some(if note.body != record.receipt().unwrap().confirmed_body {
                                    DeleteStatus::Conflict {
                                        website_body: note.body.clone(),
                                    }
                                } else {
                                    DeleteStatus::Failed(
                                        "Comment still exists; retry deletion explicitly".into(),
                                    )
                                });
                        }
                        if let Some(sent_body) =
                            record.edit.as_ref().and_then(|edit| match &edit.status {
                                EditStatus::Unknown { sent_body, .. }
                                | EditStatus::Sending { sent_body } => Some(sent_body.clone()),
                                _ => None,
                            })
                        {
                            if note.body == sent_body {
                                if let PublicationState::Published(receipt) = &mut record.state {
                                    receipt.confirmed_body = sent_body.clone();
                                }
                                record.edit = (local_body != sent_body).then(|| PendingEdit {
                                    body: local_body.into(),
                                    status: EditStatus::Pending,
                                });
                            } else {
                                record.edit = Some(PendingEdit {
                                    body: local_body.into(),
                                    status: EditStatus::Conflict {
                                        website_body: note.body.clone(),
                                    },
                                });
                            }
                        }
                    }
                }
                self.store.save(comparison, origin, id, &record)?;
                let remote = RemoteRead {
                    confirmed_body: record.receipt().unwrap().confirmed_body.clone(),
                    ..remote
                };
                self.reconcile(comparison, origin, id, local_body, editing, remote)
            }
            CheckedPublication::Create {
                operation_id,
                candidates,
            } => {
                let mut record = self
                    .store
                    .load(comparison, origin, id)?
                    .context("No publication intent is available")?;
                if record.operation_id != operation_id || record.receipt().is_some() {
                    return Ok(RefreshOutcome {
                        record,
                        adopt_body: None,
                    });
                }
                record.recovery_candidates = candidates;
                if record.recovery_candidates.len() != 1 {
                    record.state = PublicationState::Unknown(
                        if record.recovery_candidates.is_empty() {
                            "No exact marker match found; absence is not proof creation failed"
                                .into()
                        } else {
                            "Multiple marker matches found; inspect candidates before explicitly republishing".into()
                        },
                    );
                    self.store.save(comparison, origin, id, &record)?;
                    return Ok(RefreshOutcome {
                        record,
                        adopt_body: None,
                    });
                }
                let mut receipt = record.recovery_candidates[0].clone();
                let remote = RemoteNote {
                    body: receipt.confirmed_body.clone(),
                    position: receipt.remote_position.clone(),
                    resolved: receipt.resolved,
                    outdated: receipt.outdated,
                };
                receipt.confirmed_body = record.body.clone();
                record.state = PublicationState::Published(receipt);
                record.create_queued = false;
                self.store.save(comparison, origin, id, &record)?;
                let confirmed_body = record.body;
                self.reconcile(
                    comparison,
                    origin,
                    id,
                    local_body,
                    editing,
                    RemoteRead {
                        operation_id: record.operation_id,
                        note: Some(remote),
                        confirmed_body,
                    },
                )
            }
        }
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
        if record.deletion == Some(DeleteStatus::Confirmed) {
            return Ok(RefreshOutcome { record, adopt_body });
        }
        if record.operation_id != remote.operation_id
            || record.receipt().unwrap().confirmed_body != remote.confirmed_body
        {
            return Ok(RefreshOutcome { record, adopt_body });
        }
        match remote.note {
            None => record.website_deleted = true,
            Some(remote) => {
                record.observe_remote(&remote);
                let confirmed = record.receipt().unwrap().confirmed_body.clone();
                let busy = record.deletion.is_some()
                    || record.edit.as_ref().is_some_and(|edit| {
                        matches!(
                            edit.status,
                            EditStatus::Sending { .. }
                                | EditStatus::Unknown { .. }
                                | EditStatus::Conflict { .. }
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
        let mut record = self
            .store
            .load(comparison, origin, id)?
            .context("No publication intent is available")?;
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
        if body
            == record
                .receipt()
                .map(|receipt| receipt.confirmed_body.as_str())
                .unwrap_or(&record.body)
            && matches!(status, EditStatus::Pending)
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
        if record.deletion.is_some()
            || record.website_deleted
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
        if record.deletion == Some(DeleteStatus::Sending) {
            record.deletion = Some(DeleteStatus::Unknown(
                "Previous deletion confirmation unavailable".into(),
            ));
            self.store.save(comparison, origin, id, &record)?;
        }
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
        if record.deletion.is_some()
            || record.website_deleted
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
        self.publish_with_cancellation(review, contexts, origin, comment_id, connection, None)
            .await
    }
    async fn publish_with_cancellation(
        &self,
        review: &Review,
        contexts: &ReviewContexts,
        origin: &ReviewOrigin,
        comment_id: u64,
        connection: &Connection,
        cancelled: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<PublicationRecord> {
        let previous = self.store.load(&review.comparison, origin, comment_id)?;
        let recreate = previous
            .as_ref()
            .is_some_and(PublicationRecord::recreate_requested);
        if let Some(record) = &previous {
            if record.create_queued && record.deletion == Some(DeleteStatus::Pending) {
                let mut cancelled = record.clone();
                cancelled.create_queued = false;
                cancelled.deletion = Some(DeleteStatus::Confirmed);
                cancelled.state =
                    PublicationState::Failed("Create cancelled before sending".into());
                self.store
                    .save(&review.comparison, origin, comment_id, &cancelled)?;
                return Ok(cancelled);
            }
            if matches!(
                record.deletion,
                Some(DeleteStatus::Sending | DeleteStatus::Unknown(_))
            ) || record.edit.as_ref().is_some_and(|edit| {
                matches!(
                    edit.status,
                    EditStatus::Sending { .. } | EditStatus::Unknown { .. }
                )
            }) {
                bail!("A previous write remains uncertain; check GitLab before another create");
            }
            match record.state {
                PublicationState::Published(_) if !recreate => return Ok(record.clone()),
                PublicationState::Published(_) => {}
                PublicationState::Unknown(_) if !record.create_queued => {
                    bail!("Result unknown; check GitLab before another create")
                }
                PublicationState::Unknown(_) => {}
                PublicationState::Failed(_) | PublicationState::Unsupported(_) => {}
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
        let mut record = PublicationRecord::fresh_create(comment, previous.as_ref(), false);
        self.save_create(&review.comparison, origin, comment_id, &mut record)?;
        if !connection.matches(origin) || connection.pat.trim().is_empty() {
            record.state = PublicationState::Failed(if connection.pat.trim().is_empty() {
                "No GitLab token configured; add a token with api scope".into()
            } else {
                "Current connection does not match the Review's GitLab host; reconnect before retrying".into()
            });
            self.save_create(&review.comparison, origin, comment_id, &mut record)?;
            return Ok(record);
        }
        let author = self.adapter.author(origin, connection).await;
        match author {
            Ok(author) if record.author_id.is_none_or(|expected| expected == author) => {
                record.author_id = Some(author);
                self.save_create(&review.comparison, origin, comment_id, &mut record)?;
            }
            Ok(_) => {
                record.state = PublicationState::Failed(
                    "GitLab account changed; reconnect the original account before retrying".into(),
                );
                self.save_create(&review.comparison, origin, comment_id, &mut record)?;
                return Ok(record);
            }
            Err(message) => {
                record.state = PublicationState::Failed(message);
                self.save_create(&review.comparison, origin, comment_id, &mut record)?;
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
                record.state = match message {
                    crate::gitlab_publication::PreparationError::Failed(message) => {
                        PublicationState::Failed(message)
                    }
                    crate::gitlab_publication::PreparationError::Unsupported(message) => {
                        PublicationState::Unsupported(message)
                    }
                };
                self.save_create(&review.comparison, origin, comment_id, &mut record)?;
                return Ok(record);
            }
        }
        self.save_create(&review.comparison, origin, comment_id, &mut record)?;
        if record.deletion == Some(DeleteStatus::Pending) {
            record.state = PublicationState::Failed("Create cancelled before sending".into());
            self.save_create(&review.comparison, origin, comment_id, &mut record)?;
            return Ok(record);
        }
        if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst)) {
            record.state =
                PublicationState::Failed("Batch stopped before sending; retry explicitly".into());
            self.save_create(&review.comparison, origin, comment_id, &mut record)?;
            return Ok(record);
        }
        record.state = self.adapter.create(origin, connection, &record).await;
        self.save_create(&review.comparison, origin, comment_id, &mut record)?;
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
    fn verified_absence_after_lost_update_keeps_sent_evidence_and_requires_explicit_recreate() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let lost_http = FakeHttpClient::create(|request| async move {
            if request.method() == http::Method::PUT {
                return Ok(http::Response::builder()
                    .status(500)
                    .body(AsyncBody::from("lost"))
                    .unwrap());
            }
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            Ok(response(
                serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
            ))
        });
        let service = PublicationService::new(lost_http, store.clone());
        open.begin_edit(1);
        open.commit("sent B");
        service
            .queue_edit(&comparison(), &origin(), 1, "sent B")
            .unwrap();
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        futures::executor::block_on(service.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        open.begin_edit(1);
        open.commit("newer C");
        service
            .queue_edit(&comparison(), &origin(), 1, "newer C")
            .unwrap();
        let absent_http = FakeHttpClient::create(|request| async move {
            assert_eq!(request.method(), http::Method::GET);
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.uri().path().ends_with("/discussion-1") {
                return Ok(http::Response::builder()
                    .status(404)
                    .body(AsyncBody::from("absent"))
                    .unwrap());
            }
            Ok(response(serde_json::json!([])))
        });
        let reader = PublicationService::new(absent_http, store.clone());
        let checked = futures::executor::block_on(reader.check_again(
            &comparison(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        let result = reader
            .reconcile_check(&comparison(), &origin(), 1, "newer C", false, checked)
            .unwrap();
        assert!(result.record.website_deleted);
        assert!(matches!(
            result.record.edit.unwrap().status,
            EditStatus::Failed(_)
        ));
        assert_eq!(result.record.absent_updates[0].sent_body, "sent B");
        assert_eq!(result.record.absent_updates[0].receipt.note_id, 51);
        assert_eq!(open.review().comments[0].body, "newer C");
        let reserved = reader.queue_create(open.review(), &origin(), 1).unwrap();
        assert!(reserved.create_queued);
        assert_eq!(reserved.body, "newer C");
        assert_eq!(reserved.absent_updates[0].sent_body, "sent B");
        let creating = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            assert_eq!(request.method(), http::Method::POST);
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(payload["body"].as_str().unwrap().starts_with("newer C"));
            Ok(response(
                serde_json::json!({"id":"new-thread","notes":[{"id":99,"body":payload["body"],"author":{"id":42}}]}),
            ))
        });
        let created =
            futures::executor::block_on(PublicationService::new(creating, store.clone()).publish(
                open.review(),
                open.contexts(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
        assert_eq!(created.receipt().unwrap().note_id, 99);
        assert_eq!(created.absent_updates[0].sent_body, "sent B");
        assert_eq!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .absent_updates[0]
                .receipt
                .note_id,
            51
        );
    }

    #[test]
    fn a_late_check_for_the_old_native_comment_cannot_modify_a_recreated_counterpart() {
        let directory = tempfile::tempdir().unwrap();
        let (open, store) = published(directory.path());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let stale_reader = PublicationService::new(website_client("website D"), store.clone());
        let stale = futures::executor::block_on(stale_reader.check_again(
            &comparison(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        let absent = FakeHttpClient::create(|request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.uri().path().ends_with("/discussion-1") {
                return Ok(http::Response::builder()
                    .status(404)
                    .body(AsyncBody::from("missing"))
                    .unwrap());
            }
            Ok(response(serde_json::json!([])))
        });
        let absence = PublicationService::new(absent, store.clone());
        let read =
            futures::executor::block_on(absence.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        absence
            .reconcile(&comparison(), &origin(), 1, "please explain", false, read)
            .unwrap();
        let created_http = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            Ok(response(
                serde_json::json!({"id":"new-thread","notes":[{"id":99,"body":payload["body"],"author":{"id":42}}]}),
            ))
        });
        let publisher = PublicationService::new(created_http, store.clone());
        publisher.queue_create(open.review(), &origin(), 1).unwrap();
        futures::executor::block_on(publisher.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        let outcome = stale_reader
            .reconcile_check(&comparison(), &origin(), 1, "please explain", false, stale)
            .unwrap();
        assert_eq!(outcome.record.receipt().unwrap().note_id, 99);
        assert_eq!(
            outcome.record.receipt().unwrap().confirmed_body,
            "please explain"
        );
        assert!(outcome.adopt_body.is_none());
    }

    #[test]
    fn checking_unknown_delete_requires_verified_absence_and_keeps_reads_readonly() {
        for outcome in ["absent", "present", "denied"] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let lost_http = FakeHttpClient::create(|request| async move {
                if request.method() == http::Method::DELETE {
                    return Ok(http::Response::builder()
                        .status(500)
                        .body(AsyncBody::from("lost"))
                        .unwrap());
                }
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                ))
            });
            let service = PublicationService::new(lost_http, store.clone());
            service
                .queue_delete(&comparison(), &origin(), 1, "please explain")
                .unwrap();
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            let lost = futures::executor::block_on(service.delete(
                &comparison(),
                &origin(),
                1,
                &connection,
                false,
            ))
            .unwrap();
            assert!(matches!(lost.deletion, Some(DeleteStatus::Unknown(_))));
            assert!(
                service
                    .cancel_pending_delete(&comparison(), &origin(), 1)
                    .is_err()
            );
            let checking = FakeHttpClient::create(move |request| async move {
                assert_eq!(request.method(), http::Method::GET);
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if outcome == "denied" {
                    return Ok(http::Response::builder()
                        .status(403)
                        .body(AsyncBody::from("denied"))
                        .unwrap());
                }
                if outcome == "present" {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                    ));
                }
                if request.uri().path().ends_with("/discussion-1") {
                    return Ok(http::Response::builder()
                        .status(404)
                        .body(AsyncBody::from("missing"))
                        .unwrap());
                }
                assert!(request.uri().path().ends_with("/discussions"));
                Ok(response(serde_json::json!([])))
            });
            let reader = PublicationService::new(checking, store.clone());
            let checked = futures::executor::block_on(reader.check_again(
                &comparison(),
                &origin(),
                1,
                &connection,
            ));
            if outcome == "denied" {
                assert!(checked.is_err());
                assert!(matches!(
                    store
                        .load(&comparison(), &origin(), 1)
                        .unwrap()
                        .unwrap()
                        .deletion,
                    Some(DeleteStatus::Unknown(_))
                ));
            } else {
                let result = reader
                    .reconcile_check(
                        &comparison(),
                        &origin(),
                        1,
                        "please explain",
                        false,
                        checked.unwrap(),
                    )
                    .unwrap();
                if outcome == "absent" {
                    assert_eq!(result.record.deletion, Some(DeleteStatus::Confirmed));
                    open.complete_delete(1);
                    assert!(open.review().comments.is_empty());
                } else {
                    assert!(matches!(
                        result.record.deletion,
                        Some(DeleteStatus::Failed(_))
                    ));
                    assert_eq!(open.review().comments.len(), 1);
                }
            }
        }
    }

    #[test]
    fn checking_unknown_update_confirms_sent_b_preserves_c_or_exposes_website_conflict() {
        for website in ["sent B", "website D"] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let lost_http = FakeHttpClient::create(|request| async move {
                if request.method() == http::Method::PUT {
                    return Ok(http::Response::builder()
                        .status(500)
                        .body(AsyncBody::from("lost"))
                        .unwrap());
                }
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                ))
            });
            let service = PublicationService::new(lost_http, store.clone());
            open.begin_edit(1);
            open.commit("sent B");
            service
                .queue_edit(&comparison(), &origin(), 1, "sent B")
                .unwrap();
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            let lost = futures::executor::block_on(service.update(
                &comparison(),
                &origin(),
                1,
                &connection,
                false,
            ))
            .unwrap();
            assert!(matches!(
                lost.edit.as_ref().unwrap().status,
                EditStatus::Unknown { .. }
            ));
            open.begin_edit(1);
            open.commit("newer C");
            service
                .queue_edit(&comparison(), &origin(), 1, "newer C")
                .unwrap();
            let reader = PublicationService::new(
                website_client(&format!("{website}\n\n{}", lost.marker())),
                store.clone(),
            );
            let checked = futures::executor::block_on(reader.check_again(
                &comparison(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
            let outcome = reader
                .reconcile_check(&comparison(), &origin(), 1, "newer C", false, checked)
                .unwrap();
            let edit = outcome.record.edit.as_ref().unwrap();
            assert_eq!(edit.body, "newer C");
            if website == "sent B" {
                assert_eq!(outcome.record.receipt().unwrap().confirmed_body, "sent B");
                assert_eq!(edit.status, EditStatus::Pending);
            } else {
                assert!(
                    matches!(&edit.status, EditStatus::Conflict {website_body} if website_body == "website D")
                );
            }
            assert!(outcome.adopt_body.is_none());
            assert_eq!(open.review().comments[0].body, "newer C");
        }
    }

    #[test]
    fn zero_and_multiple_matches_require_duplicate_confirmation_and_retain_old_attempts() {
        for count in [0, 2] {
            let directory = tempfile::tempdir().unwrap();
            let open = drafted(directory.path(), Side::Postimage, 2);
            let store = PublicationStore::new(directory.path().join("remote"));
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            let losing = FakeHttpClient::create(|request| async move {
                if let Some(value) = preflight(request.uri().path()) {
                    return Ok(response(value));
                }
                Ok(http::Response::builder()
                    .status(500)
                    .body(AsyncBody::from("lost"))
                    .unwrap())
            });
            let lost = futures::executor::block_on(
                PublicationService::new(losing, store.clone()).publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &connection,
                ),
            )
            .unwrap();
            let body = lost.remote_body();
            let client = FakeHttpClient::create(move |request| {
                let body = body.clone();
                async move {
                    assert_eq!(request.method(), http::Method::GET);
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    let candidates: Vec<_> = (0..count).map(|n| serde_json::json!({"id":format!("thread-{n}"),"notes":[{"id":70+n,"body":body,"author":{"id":42}}]})).collect();
                    Ok(response(serde_json::json!(candidates)))
                }
            });
            let service = PublicationService::new(client, store.clone());
            service
                .queue_delete(&comparison(), &origin(), 1, "please explain")
                .unwrap();
            let checked = futures::executor::block_on(service.check_again(
                &comparison(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
            let result = service
                .reconcile_check(
                    &comparison(),
                    &origin(),
                    1,
                    "please explain",
                    false,
                    checked,
                )
                .unwrap();
            assert!(matches!(result.record.state, PublicationState::Unknown(_)));
            assert_eq!(result.record.recovery_candidates.len(), count as usize);
            assert!(
                service
                    .queue_republish(open.review(), &origin(), 1, false)
                    .is_err()
            );
            assert_eq!(
                store
                    .load(&comparison(), &origin(), 1)
                    .unwrap()
                    .unwrap()
                    .operation_id,
                lost.operation_id
            );
            assert!(
                service
                    .queue_republish(open.review(), &origin(), 1, true)
                    .is_err()
            );
            let kept = service
                .cancel_pending_delete(&comparison(), &origin(), 1)
                .unwrap();
            assert!(matches!(kept.state, PublicationState::Unknown(_)));
            assert_eq!(kept.operation_id, lost.operation_id);
            assert_eq!(kept.marker(), lost.marker());
            assert_eq!(kept.recovery_candidates, result.record.recovery_candidates);
            assert!(kept.deletion.is_none());
            assert!(kept.deletion_body.is_none());
            let reopened = OpenReview::reopen(
                comparison(),
                "a.rs",
                origin(),
                ReviewStore::new(directory.path().join("local")),
            )
            .with_publication_store(store.clone());
            assert_eq!(reopened.review().comments.len(), 1);
            assert!(
                service
                    .queue_republish(reopened.review(), &origin(), 1, false)
                    .is_err()
            );
            let reserved = service
                .queue_republish(reopened.review(), &origin(), 1, true)
                .unwrap();
            assert_ne!(reserved.operation_id, lost.operation_id);
            assert!(reserved.create_queued);
            assert_eq!(reserved.prior_attempts.len(), 1);
            assert_eq!(reserved.prior_attempts[0].operation_id, lost.operation_id);
            assert_eq!(reserved.prior_attempts[0].candidates.len(), count as usize);
            let old_visible = reserved.visible_body(&lost.remote_body());
            assert_eq!(old_visible, "please explain");
            let success = FakeHttpClient::create(|mut request| async move {
                if let Some(value) = preflight(request.uri().path()) {
                    return Ok(response(value));
                }
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                Ok(response(
                    serde_json::json!({"id":"new-thread","notes":[{"id":99,"body":payload["body"],"author":{"id":42}}]}),
                ))
            });
            let published = futures::executor::block_on(
                PublicationService::new(success, store.clone()).publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &connection,
                ),
            )
            .unwrap();
            assert_eq!(published.receipt().unwrap().note_id, 99);
            assert_eq!(published.prior_attempts[0].operation_id, lost.operation_id);
            assert_eq!(
                store
                    .load(&comparison(), &origin(), 1)
                    .unwrap()
                    .unwrap()
                    .prior_attempts
                    .len(),
                1
            );
        }
    }

    #[test]
    fn checking_lost_create_pages_exact_marker_author_and_restores_b_without_syncing_c() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        let losing = FakeHttpClient::create(|request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            Ok(http::Response::builder()
                .status(500)
                .body(AsyncBody::from("lost response"))
                .unwrap())
        });
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let lost =
            futures::executor::block_on(PublicationService::new(losing, store.clone()).publish(
                open.review(),
                open.contexts(),
                &origin(),
                1,
                &connection,
            ))
            .unwrap();
        assert!(matches!(lost.state, PublicationState::Unknown(_)));
        open.begin_edit(1);
        open.commit("newer C");
        PublicationService::new(website_client("unused"), store.clone())
            .queue_edit(&comparison(), &origin(), 1, "newer C")
            .unwrap();
        PublicationService::new(website_client("unused"), store.clone())
            .queue_delete(&comparison(), &origin(), 1, "newer C")
            .unwrap();
        let body = lost.remote_body();
        let client = FakeHttpClient::create(move |request| {
            let body = body.clone();
            async move {
                assert_eq!(request.method(), http::Method::GET);
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                assert!(request.uri().path().ends_with("/discussions"));
                if request.uri().query().unwrap().ends_with("&page=1") {
                    let value = serde_json::json!([{"id":"wrong-author","notes":[{"id":60,"body":body,"author":{"id":99}}]},{"id":"changed-marker","notes":[{"id":61,"body":"please explain<!-- reviewfox:operation=changed -->","author":{"id":42}}]}]);
                    return Ok(http::Response::builder()
                        .status(200)
                        .header("x-next-page", "2")
                        .body(AsyncBody::from(value.to_string()))
                        .unwrap());
                }
                assert!(request.uri().query().unwrap().ends_with("&page=2"));
                Ok(response(
                    serde_json::json!([{"id":"original-thread","notes":[{"id":51,"body":body,"author":{"id":42},"position":{"new_line":2}}]}]),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let checked = futures::executor::block_on(service.check_again(
            &comparison(),
            &origin(),
            1,
            &connection,
        ))
        .unwrap();
        let outcome = service
            .reconcile_check(&comparison(), &origin(), 1, "newer C", false, checked)
            .unwrap();
        assert_eq!(outcome.record.receipt().unwrap().note_id, 51);
        assert_eq!(
            outcome.record.receipt().unwrap().confirmed_body,
            "please explain"
        );
        assert_eq!(outcome.record.deletion, Some(DeleteStatus::Pending));
        assert_eq!(outcome.record.edit.unwrap().body, "newer C");
        assert!(outcome.adopt_body.is_none());
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        )
        .with_publication_store(store.clone());
        assert_eq!(reopened.review().comments[0].body, "newer C");
        assert_eq!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .receipt()
                .unwrap()
                .note_id,
            51
        );
        assert!(
            !crate::export::export_review(reopened.review(), reopened.contexts())
                .contains("reviewfox:operation")
        );
    }

    #[test]
    fn cancelling_during_position_read_never_starts_a_create_after_window_close() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        let (release, waiting) = futures::channel::oneshot::channel::<()>();
        let waiting = Arc::new(std::sync::Mutex::new(Some(waiting)));
        let client = FakeHttpClient::create(move |request| {
            let waiting = waiting.clone();
            async move {
                assert_eq!(
                    request.method(),
                    http::Method::GET,
                    "A closed batch must never start POST"
                );
                if request.uri().path().ends_with("/versions/7") {
                    let wait = waiting.lock().unwrap().take().unwrap();
                    wait.await.unwrap();
                }
                Ok(response(preflight(request.uri().path()).unwrap()))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let mut batch = service
            .reserve_all(open.review(), open.contexts(), &origin())
            .unwrap();
        let cancel = batch.cancellation();
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let mut preparing = Box::pin(batch.publish_next(&service, &connection));
        assert!(preparing.as_mut().now_or_never().is_none());
        cancel.cancel();
        release.send(()).unwrap();
        let (_, record) = futures::executor::block_on(preparing).unwrap().unwrap();
        assert!(matches!(record.state, PublicationState::Failed(_)));
        assert_eq!(
            service.draft_ids(open.review(), &origin()).unwrap(),
            vec![1]
        );
    }

    #[test]
    fn batch_cancellation_preserves_inflight_receipt_latest_edit_and_queued_delete() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context(
            "unchanged\ntail\n".into(),
            "unchanged\nadded\ntail\n".into(),
        );
        open.commit("delete me");
        let store = PublicationStore::new(directory.path().join("remote"));
        let (release, waiting) = futures::channel::oneshot::channel::<()>();
        let waiting = Arc::new(std::sync::Mutex::new(Some(waiting)));
        let client = FakeHttpClient::create(move |mut request| {
            let waiting = waiting.clone();
            async move {
                if let Some(value) = preflight(request.uri().path()) {
                    return Ok(response(value));
                }
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert!(
                    payload["body"]
                        .as_str()
                        .unwrap()
                        .starts_with("please explain")
                );
                let wait = waiting.lock().unwrap().take().unwrap();
                wait.await.unwrap();
                Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let mut batch = service
            .reserve_all(open.review(), open.contexts(), &origin())
            .unwrap();
        let cancel = batch.cancellation();
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let mut sending = Box::pin(batch.publish_next(&service, &connection));
        assert!(sending.as_mut().now_or_never().is_none());
        open.begin_edit(1);
        open.commit("newer C");
        service
            .queue_edit(&comparison(), &origin(), 1, "newer C")
            .unwrap();
        service
            .queue_delete(&comparison(), &origin(), 2, "delete me")
            .unwrap();
        cancel.cancel();
        release.send(()).unwrap();
        let (_, created) = futures::executor::block_on(sending).unwrap().unwrap();
        assert_eq!(created.receipt().unwrap().confirmed_body, "please explain");
        assert_eq!(created.edit.unwrap().body, "newer C");
        assert!(
            futures::executor::block_on(batch.publish_next(&service, &connection))
                .unwrap()
                .is_none()
        );
        drop(batch);
        let deleted = store.load(&comparison(), &origin(), 2).unwrap().unwrap();
        assert_eq!(deleted.deletion, Some(DeleteStatus::Confirmed));
        assert!(deleted.receipt().is_none());
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        )
        .with_publication_store(store.clone());
        assert_eq!(reopened.review().comments[0].body, "newer C");
        assert_eq!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .edit
                .unwrap()
                .body,
            "newer C"
        );
    }

    #[test]
    fn batch_mixed_outcomes_retry_only_definite_failure_and_preserve_each_draft() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        for (line, body) in [(1, "denied"), (3, "uncertain"), (4, "unsupported")] {
            open.begin_draft(Side::Postimage, line, 1);
            open.capture_draft_context(
                "unchanged\ntail\n".into(),
                "unchanged\nadded\ntail\nextra\n".into(),
            );
            open.commit(body);
        }
        let store = PublicationStore::new(directory.path().join("remote"));
        let client = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let body = payload["body"].as_str().unwrap();
            assert!(!body.starts_with("unsupported"));
            if body.starts_with("denied") || body.starts_with("uncertain") {
                return Ok(http::Response::builder()
                    .status(if body.starts_with("denied") { 403 } else { 500 })
                    .body(AsyncBody::from("rejected"))
                    .unwrap());
            }
            Ok(response(
                serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
            ))
        });
        let service = PublicationService::new(client, store.clone());
        let mut batch = service
            .reserve_all(open.review(), open.contexts(), &origin())
            .unwrap();
        assert_eq!(batch.remaining(), 4);
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let mut outcomes = Vec::new();
        while let Some((_, record)) =
            futures::executor::block_on(batch.publish_next(&service, &connection)).unwrap()
        {
            outcomes.push(record);
        }
        assert!(outcomes[0].receipt().is_some());
        assert!(matches!(outcomes[1].state, PublicationState::Failed(_)));
        assert!(matches!(outcomes[2].state, PublicationState::Unknown(_)));
        assert!(matches!(
            outcomes[3].state,
            PublicationState::Unsupported(_)
        ));
        assert!(outcomes[3].status_label().contains("unavailable"));
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        )
        .with_publication_store(store.clone());
        assert_eq!(reopened.review().comments.len(), 4);
        assert_eq!(
            service.draft_ids(reopened.review(), &origin()).unwrap(),
            vec![2]
        );
        let retry_http = FakeHttpClient::create(|mut request| async move {
            if let Some(value) = preflight(request.uri().path()) {
                return Ok(response(value));
            }
            let mut bytes = Vec::new();
            request.body_mut().read_to_end(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(payload["body"].as_str().unwrap().starts_with("denied"));
            Ok(response(
                serde_json::json!({"id":"discussion-2","notes":[{"id":52,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
            ))
        });
        let retry_service = PublicationService::new(retry_http, store.clone());
        let mut retry = retry_service
            .reserve_all(reopened.review(), reopened.contexts(), &origin())
            .unwrap();
        let (id, record) =
            futures::executor::block_on(retry.publish_next(&retry_service, &connection))
                .unwrap()
                .unwrap();
        assert_eq!(id, 2);
        assert_eq!(record.receipt().unwrap().note_id, 52);
        assert!(
            futures::executor::block_on(retry.publish_next(&retry_service, &connection))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .receipt()
                .unwrap()
                .note_id,
            51
        );
    }

    #[test]
    fn closing_batch_preserves_success_and_leaves_unsent_drafts_for_manual_retry() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        open.begin_draft(Side::Postimage, 1, 1);
        open.capture_draft_context(
            "unchanged\ntail\n".into(),
            "unchanged\nadded\ntail\n".into(),
        );
        open.commit("second draft");
        let store = PublicationStore::new(directory.path().join("remote"));
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
        let mut batch = service
            .reserve_all(open.review(), open.contexts(), &origin())
            .unwrap();
        assert_eq!(batch.remaining(), 2);
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let (id, record) = futures::executor::block_on(batch.publish_next(&service, &connection))
            .unwrap()
            .unwrap();
        assert_eq!(id, 1);
        assert!(record.receipt().is_some());
        drop(batch);
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        )
        .with_publication_store(store.clone());
        assert_eq!(reopened.review().comments.len(), 2);
        assert!(
            store
                .load(&comparison(), &origin(), 1)
                .unwrap()
                .unwrap()
                .receipt()
                .is_some()
        );
        let unsent = store.load(&comparison(), &origin(), 2).unwrap().unwrap();
        assert!(matches!(unsent.state, PublicationState::Failed(_)));
        assert!(!unsent.create_queued);
        assert_eq!(
            service.draft_ids(reopened.review(), &origin()).unwrap(),
            vec![2]
        );
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
    fn rename_disabled_add_delete_contexts_fetch_only_missing_historical_verification_blobs() {
        for (side, deny) in [
            (Side::Postimage, false),
            (Side::Preimage, false),
            (Side::Postimage, true),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = match side {
                Side::Preimage => "old.rs",
                Side::Postimage => "new.rs",
            };
            let mut open = OpenReview::reopen(
                comparison(),
                path,
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(side, 1, 2);
            let (pre, post) = match side {
                Side::Preimage => ("same\nold\ntail\n", ""),
                Side::Postimage => ("", "same\nnew\ntail\n"),
            };
            open.capture_draft_context(pre.into(), post.into());
            open.commit("rename mixed range");
            let before = open.contexts().clone();
            let exported = crate::export::export_review(open.review(), open.contexts());
            let mut fixture = version();
            fixture["diffs"][0] = serde_json::json!({"old_path":"old.rs","new_path":"new.rs","renamed_file":true,"diff":"@@ -1,3 +1,3 @@\n same\n-old\n+new\n tail\n"});
            let client = FakeHttpClient::create(move |mut request| {
                let fixture = fixture.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        return Ok(response(serde_json::json!([fixture])));
                    }
                    if request.uri().path().ends_with("/versions/7") {
                        return Ok(response(fixture));
                    }
                    if request.uri().path().contains("/repository/files/") {
                        assert_eq!(request.method(), http::Method::GET);
                        let (path, sha, text) = match side {
                            Side::Postimage => (
                                "old.rs",
                                Oid::from_bytes([1; 20]).to_string(),
                                "same\nold\ntail\n",
                            ),
                            Side::Preimage => (
                                "new.rs",
                                Oid::from_bytes([2; 20]).to_string(),
                                "same\nnew\ntail\n",
                            ),
                        };
                        assert_eq!(
                            request.uri().path(),
                            format!("/api/v4/projects/team%2Frepo/repository/files/{path}/raw")
                        );
                        assert_eq!(request.uri().query(), Some(format!("ref={sha}").as_str()));
                        return Ok(http::Response::builder()
                            .status(if deny { 403 } else { 200 })
                            .body(AsyncBody::from(text))
                            .unwrap());
                    }
                    assert!(
                        !deny,
                        "Do not publish without the historical verification blob"
                    );
                    assert_eq!(request.method(), http::Method::POST);
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(
                        payload["position"]["line_range"]["start"]["type"],
                        serde_json::Value::Null
                    );
                    assert_eq!(
                        payload["position"]["line_range"]["end"]["type"],
                        match side {
                            Side::Preimage => "old",
                            Side::Postimage => "new",
                        }
                    );
                    Ok(response(
                        serde_json::json!({"id":"rename-thread","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                    ))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            if deny {
                assert!(record.status_label().contains("403"));
            } else {
                assert!(record.receipt().is_some(), "{}", record.status_label());
            }
            assert_eq!(open.contexts(), &before);
            assert_eq!(
                crate::export::export_review(open.review(), open.contexts()),
                exported
            );
        }
    }

    #[test]
    fn collapsed_version_retrieval_stays_pinned_and_reports_distinct_file_limits() {
        for (case, reason) in [
            ("collapsed_success", None),
            ("collapsed_missing", Some("collapsed")),
            ("too_large", Some("too_large")),
            ("rename_only", Some("Rename-only")),
            ("empty", Some("no text diff")),
            ("overflow", Some("file/line limits")),
            ("changed_version", Some("different version")),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut open = OpenReview::reopen(
                comparison(),
                "a.rs",
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(Side::Postimage, 1, 2);
            open.capture_draft_context(
                "unchanged\ntail\n".into(),
                "unchanged\nadded\ntail\n".into(),
            );
            open.commit("retained range");
            let mut fixture = version();
            match case {
                "collapsed_success" | "collapsed_missing" | "changed_version" => {
                    fixture["diffs"][0]["collapsed"] = true.into();
                    fixture["diffs"][0]["diff"] = "".into();
                }
                "too_large" => {
                    fixture["diffs"][0]["too_large"] = true.into();
                    fixture["diffs"][0]["diff"] = "".into();
                }
                "rename_only" => {
                    fixture["diffs"][0]["renamed_file"] = true.into();
                    fixture["diffs"][0]["old_path"] = "old.rs".into();
                    fixture["diffs"][0]["diff"] = "".into();
                }
                "empty" => {
                    fixture["diffs"][0].as_object_mut().unwrap().remove("diff");
                }
                "overflow" => {
                    fixture["state"] = "overflow".into();
                    fixture["diffs"] = serde_json::json!([]);
                }
                _ => unreachable!(),
            }
            let client = FakeHttpClient::create(move |mut request| {
                let fixture = fixture.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        return Ok(response(serde_json::json!([fixture])));
                    }
                    if request.uri().path().ends_with("/versions/7") {
                        if request.uri().query() == Some("unidiff=true") {
                            assert!(matches!(
                                case,
                                "collapsed_success" | "collapsed_missing" | "changed_version"
                            ));
                            let mut unified = version();
                            if case == "collapsed_missing" {
                                unified = fixture;
                            }
                            if case == "changed_version" {
                                unified["head_commit_sha"] =
                                    Oid::from_bytes([4; 20]).to_string().into();
                            }
                            return Ok(response(unified));
                        }
                        return Ok(response(fixture));
                    }
                    assert_eq!(
                        case, "collapsed_success",
                        "Unavailable historical position must not write or fetch a current diff"
                    );
                    assert_eq!(request.method(), http::Method::POST);
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert!(payload["position"]["line_range"].is_object());
                    Ok(response(
                        serde_json::json!({"id":"unified-thread","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                    ))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            if let Some(reason) = reason {
                assert!(matches!(record.state, PublicationState::Failed(_)));
                assert!(
                    record.status_label().contains(reason),
                    "{}",
                    record.status_label()
                );
            } else {
                assert!(record.receipt().is_some());
            }
            assert_eq!(open.current().comments.len(), 1);
            assert!(
                crate::export::export_review(open.review(), open.contexts())
                    .contains("retained range")
            );
        }
    }

    #[test]
    fn cross_gitlab_sections_and_unverifiable_expanded_ranges_remain_local_without_truncation() {
        for (start, count, reason) in [
            (3, 11, "crosses GitLab diff sections"),
            (6, 6, "three-line unfolding window"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let pre = (1..=20).map(|n| format!("line{n}\n")).collect::<String>();
            let post = (1..=20)
                .flat_map(|n| {
                    let mut lines = if n == 12 {
                        vec![]
                    } else {
                        vec![format!("line{n}\n")]
                    };
                    if n == 2 {
                        lines.extend(["addedA\n".into(), "addedB\n".into()]);
                    }
                    lines
                })
                .collect::<String>();
            let mut open = OpenReview::reopen(
                comparison(),
                "a.rs",
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(Side::Postimage, start, count);
            open.capture_draft_context(pre.into(), post.into());
            open.commit("unavailable range");
            let original = open.current().comments[0].anchor.clone();
            let mut fixture = version();
            fixture["diffs"][0]["diff"]="@@ -1,3 +1,5 @@\n line1\n line2\n+addedA\n+addedB\n line3\n@@ -11,3 +13,2 @@\n line11\n-line12\n line13\n".into();
            let client = FakeHttpClient::create(move |request| {
                let fixture = fixture.clone();
                async move {
                    assert_eq!(
                        request.method(),
                        http::Method::GET,
                        "Do not split, collapse or publish a partial selection"
                    );
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        return Ok(response(serde_json::json!([fixture])));
                    }
                    assert!(request.uri().path().ends_with("/versions/7"));
                    Ok(response(fixture))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            assert!(
                record.status_label().contains(reason),
                "{}",
                record.status_label()
            );
            assert_eq!(open.current().comments[0].anchor, original);
            assert!(
                crate::export::export_review(open.review(), open.contexts())
                    .contains(&format!("L{start}-{}", start + count - 1))
            );
        }
    }

    #[test]
    fn renamed_whitespace_only_historical_ranges_use_original_refs_and_server_tracked_placement() {
        for (side, path) in [
            (Side::Preimage, "new.rs"),
            (Side::Preimage, "old.rs"),
            (Side::Postimage, "new.rs"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut open = OpenReview::reopen(
                comparison(),
                path,
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(side, 2, 2);
            open.capture_draft_context(
                "same\n old1\n old2\ntail\n".into(),
                "same\nold1\nold2\ntail\n".into(),
            );
            open.commit("whitespace range");
            let before = open.current().comments[0].anchor.clone();
            let mut fixture = version();
            fixture["diffs"][0] = serde_json::json!({"old_path":"old.rs","new_path":"new.rs","renamed_file":true,"diff":"@@ -1,4 +1,4 @@\n same\n- old1\n- old2\n+old1\n+old2\n tail\n"});
            let client = FakeHttpClient::create(move |mut request| {
                let fixture = fixture.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        let mut current = fixture.clone();
                        current["id"] = 8.into();
                        current["head_commit_sha"] = Oid::from_bytes([4; 20]).to_string().into();
                        return Ok(response(serde_json::json!([current, fixture])));
                    }
                    if request.uri().path().ends_with("/versions/7") {
                        return Ok(response(fixture));
                    }
                    assert_eq!(request.method(), http::Method::POST);
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(
                        payload["position"]["base_sha"],
                        Oid::from_bytes([1; 20]).to_string()
                    );
                    assert_eq!(
                        payload["position"]["head_sha"],
                        Oid::from_bytes([2; 20]).to_string()
                    );
                    assert_eq!(
                        payload["position"]["start_sha"],
                        Oid::from_bytes([3; 20]).to_string()
                    );
                    assert_eq!(payload["position"]["old_path"], "old.rs");
                    assert_eq!(payload["position"]["new_path"], "new.rs");
                    let (start_code, end_code, kind) = match side {
                        Side::Preimage => (
                            "e6d049e7e635307aff069ed13eadd2e56f36bc52_2_2",
                            "e6d049e7e635307aff069ed13eadd2e56f36bc52_3_2",
                            "old",
                        ),
                        Side::Postimage => (
                            "e6d049e7e635307aff069ed13eadd2e56f36bc52_4_2",
                            "e6d049e7e635307aff069ed13eadd2e56f36bc52_4_3",
                            "new",
                        ),
                    };
                    assert_eq!(
                        payload["position"]["line_range"]["start"]["line_code"],
                        start_code
                    );
                    assert_eq!(
                        payload["position"]["line_range"]["end"]["line_code"],
                        end_code
                    );
                    assert_eq!(payload["position"]["line_range"]["end"]["type"], kind);
                    Ok(response(
                        serde_json::json!({"id":"tracked-thread","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":{"new_path":"new.rs","head_sha":Oid::from_bytes([4;20]).to_string(),"new_line":77},"resolved":true}]}),
                    ))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            let receipt = record
                .receipt()
                .unwrap_or_else(|| panic!("{}", record.status_label()));
            assert_eq!(receipt.remote_position["new_line"], 77);
            assert_eq!(receipt.resolved, Some(true));
            assert_eq!(receipt.outdated, None);
            assert!(receipt.placement_label().contains("unavailable"));
            assert_eq!(open.current().comments[0].anchor, before);
            assert_eq!(record.status_label(), "Published on GitLab");
        }
    }

    #[test]
    fn expanded_context_ranges_use_each_gap_offset_and_validate_both_captured_blobs() {
        let pre = (1..=20).map(|n| format!("line{n}\n")).collect::<String>();
        let post = (1..=20)
            .flat_map(|n| {
                let mut lines = if n == 12 {
                    Vec::new()
                } else {
                    vec![format!("line{n}\n")]
                };
                if n == 2 {
                    lines.extend(["addedA\n".into(), "addedB\n".into()]);
                }
                lines
            })
            .collect::<String>();
        for (start, old_start, valid) in [(8, 6, true), (17, 16, true), (8, 6, false)] {
            let directory = tempfile::tempdir().unwrap();
            let mut open = OpenReview::reopen(
                comparison(),
                "a.rs",
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(Side::Postimage, start, 2);
            open.capture_draft_context(
                pre.clone().into(),
                if valid {
                    post.clone()
                } else {
                    post.replace("line6\n", "changed6\n")
                }
                .into(),
            );
            open.commit("expanded body");
            let mut fixture = version();
            fixture["diffs"][0]["diff"]="@@ -1,3 +1,5 @@\n line1\n line2\n+addedA\n+addedB\n line3\n@@ -11,3 +13,2 @@\n line11\n-line12\n line13\n".into();
            let client = FakeHttpClient::create(move |mut request| {
                let fixture = fixture.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        return Ok(response(serde_json::json!([fixture])));
                    }
                    if request.uri().path().ends_with("/versions/7") {
                        return Ok(response(fixture));
                    }
                    assert!(valid, "Invalid captured context must not write");
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(payload["position"]["old_line"], old_start + 1);
                    assert_eq!(payload["position"]["new_line"], start + 1);
                    assert_eq!(
                        payload["position"]["line_range"]["start"]["line_code"],
                        format!("0973b0b779533bb9fb584987e16ebcddbfe706f1_{old_start}_{start}")
                    );
                    assert_eq!(
                        payload["position"]["line_range"]["end"]["line_code"],
                        format!(
                            "0973b0b779533bb9fb584987e16ebcddbfe706f1_{}_{}",
                            old_start + 1,
                            start + 1
                        )
                    );
                    Ok(response(
                        serde_json::json!({"id":"expanded-thread","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                    ))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            if valid {
                assert!(record.receipt().is_some(), "{}", record.status_label());
            } else {
                assert!(record.status_label().contains("captured"));
            }
            assert!(
                crate::export::export_review(open.review(), open.contexts())
                    .contains("expanded body")
            );
        }
    }

    #[test]
    fn publish_preserves_added_deleted_and_context_range_endpoints_in_gitlab_raw_section() {
        for (side, start, pre, post, patch, expected) in [
            (
                Side::Postimage,
                2,
                "same\ntail\n",
                "same\nnew1\nnew2\ntail\n",
                "@@ -1,2 +1,4 @@\n same\n+new1\n+new2\n tail\n",
                serde_json::json!({"start":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_2_2","type":"new","new_line":2},"end":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_2_3","type":"new","new_line":3}}),
            ),
            (
                Side::Preimage,
                2,
                "same\nold1\nold2\ntail\n",
                "same\ntail\n",
                "@@ -1,4 +1,2 @@\n same\n-old1\n-old2\n tail\n",
                serde_json::json!({"start":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_2_2","type":"old","old_line":2},"end":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_3_2","type":"old","old_line":3}}),
            ),
            (
                Side::Postimage,
                1,
                "same1\nsame2\ntail\n",
                "same1\nsame2\nnew\ntail\n",
                "@@ -1,3 +1,4 @@\n same1\n same2\n+new\n tail\n",
                serde_json::json!({"start":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_1_1","type":null,"old_line":1,"new_line":1},"end":{"line_code":"0973b0b779533bb9fb584987e16ebcddbfe706f1_2_2","type":null,"old_line":2,"new_line":2}}),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut open = OpenReview::reopen(
                comparison(),
                "a.rs",
                origin(),
                ReviewStore::new(directory.path().join("local")),
            );
            open.begin_draft(side, start, 2);
            open.capture_draft_context(pre.into(), post.into());
            open.commit("range body");
            let before = open.current().comments[0].anchor.clone();
            let mut fixture = version();
            fixture["diffs"][0]["diff"] = patch.into();
            let client = FakeHttpClient::create(move |mut request| {
                let fixture = fixture.clone();
                let expected = expected.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.uri().path().ends_with("/versions") {
                        return Ok(response(serde_json::json!([fixture])));
                    }
                    if request.uri().path().ends_with("/versions/7") {
                        return Ok(response(fixture));
                    }
                    assert_eq!(request.method(), http::Method::POST);
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(payload["position"]["line_range"], expected);
                    assert_eq!(
                        payload["position"].get("old_line"),
                        expected["end"].get("old_line")
                    );
                    assert_eq!(
                        payload["position"].get("new_line"),
                        expected["end"].get("new_line")
                    );
                    Ok(response(
                        serde_json::json!({"id":"range-thread","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                    ))
                }
            });
            let record = futures::executor::block_on(
                PublicationService::new(
                    client,
                    PublicationStore::new(directory.path().join("remote")),
                )
                .publish(
                    open.review(),
                    open.contexts(),
                    &origin(),
                    1,
                    &Connection::new("https://gitlab.example.com".into(), "secret".into()),
                ),
            )
            .unwrap();
            assert!(
                matches!(record.state, PublicationState::Published(_)),
                "{}",
                record.status_label()
            );
            assert_eq!(open.current().comments[0].anchor, before);
            assert!(
                crate::export::export_review(open.review(), open.contexts()).contains("range body")
            );
        }
    }

    #[test]
    fn successful_delete_preserves_a_later_local_save_and_active_editor() {
        use futures::FutureExt;
        for saving in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let (release, waiting) = futures::channel::oneshot::channel::<()>();
            let waiting = Arc::new(std::sync::Mutex::new(Some(waiting)));
            let client = FakeHttpClient::create(move |request| {
                let waiting = waiting.clone();
                async move {
                    if request.uri().path().ends_with("/user") {
                        return Ok(response(serde_json::json!({"id":42})));
                    }
                    if request.method() == http::Method::GET {
                        return Ok(response(
                            serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                        ));
                    }
                    assert_eq!(request.method(), http::Method::DELETE);
                    let wait = waiting.lock().unwrap().take().unwrap();
                    wait.await.unwrap();
                    Ok(http::Response::builder()
                        .status(204)
                        .body(AsyncBody::empty())
                        .unwrap())
                }
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            service
                .queue_delete(&comparison(), &origin(), 1, "please explain")
                .unwrap();
            let cmp = comparison();
            let target = origin();
            let mut deleting = Box::pin(service.delete(&cmp, &target, 1, &connection, false));
            assert!(deleting.as_mut().now_or_never().is_none());
            open.begin_edit(1);
            if saving {
                open.commit("newer C");
                service
                    .queue_edit(&comparison(), &origin(), 1, "newer C")
                    .unwrap();
            }
            release.send(()).unwrap();
            let deleted = futures::executor::block_on(deleting).unwrap();
            assert_eq!(deleted.deletion, Some(DeleteStatus::Confirmed));
            assert!(deleted.website_deleted);
            open.complete_delete(1);
            assert_eq!(open.current().comments.len(), 1);
            if saving {
                assert_eq!(open.current().comments[0].body, "newer C");
            } else {
                assert_eq!(open.dock().unwrap().editing, Some(1));
            }
        }
    }

    #[test]
    fn deleting_a_queued_create_is_durable_before_the_background_task_starts() {
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        open = open.with_publication_store(store.clone());
        let client = FakeHttpClient::create(|_| async {
            panic!("Cancelled create must not contact GitLab")
        });
        let service = PublicationService::new(client, store.clone());
        service.queue_create(open.review(), &origin(), 1).unwrap();
        service
            .queue_delete(&comparison(), &origin(), 1, "please explain")
            .unwrap();
        let record = futures::executor::block_on(service.publish(
            open.review(),
            open.contexts(),
            &origin(),
            1,
            &Connection::new("https://gitlab.example.com".into(), "secret".into()),
        ))
        .unwrap();
        assert_eq!(record.deletion, Some(DeleteStatus::Confirmed));
        // A response can complete after navigating away. Opening the captured
        // Review later completes local cleanup without contacting GitLab.
        drop(open);
        let open = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        )
        .with_publication_store(store.clone());
        assert!(open.current().comments.is_empty());
    }

    #[test]
    fn queued_deletion_waits_for_the_live_update_and_blocks_later_opposite_writes() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let (release, waiting) = futures::channel::oneshot::channel::<()>();
        let waiting = Arc::new(std::sync::Mutex::new(Some(waiting)));
        let client = FakeHttpClient::create(move |mut request| {
            let waiting = waiting.clone();
            async move {
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.method() == http::Method::GET {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                    ));
                }
                assert_eq!(request.method(), http::Method::PUT);
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let wait = waiting.lock().unwrap().take().unwrap();
                wait.await.unwrap();
                Ok(response(
                    serde_json::json!({"id":51,"body":payload["body"],"author":{"id":42}}),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        open.begin_edit(1);
        open.commit("body B");
        service
            .queue_edit(&comparison(), &origin(), 1, "body B")
            .unwrap();
        let cmp = comparison();
        let target = origin();
        let mut update = Box::pin(service.update(&cmp, &target, 1, &connection, false));
        assert!(update.as_mut().now_or_never().is_none());
        service
            .queue_delete(&comparison(), &origin(), 1, "please explain")
            .unwrap();
        let waiting = futures::executor::block_on(service.delete(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        assert_eq!(waiting.deletion, Some(DeleteStatus::Pending));
        release.send(()).unwrap();
        let updated = futures::executor::block_on(update).unwrap();
        assert_eq!(updated.receipt().unwrap().confirmed_body, "body B");
        assert_eq!(updated.deletion, Some(DeleteStatus::Pending));
        service
            .queue_edit(&comparison(), &origin(), 1, "newer C")
            .unwrap();
        let reader = PublicationService::new(website_client("body B"), store);
        let latest = futures::executor::block_on(reader.update(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        assert_eq!(latest.edit.unwrap().body, "newer C");
        assert_eq!(latest.deletion, Some(DeleteStatus::Pending));
    }

    #[test]
    fn deletion_queued_during_create_preserves_receipt_and_never_removes_newer_shared_work() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let mut open = drafted(directory.path(), Side::Postimage, 2);
        let store = PublicationStore::new(directory.path().join("remote"));
        open = open.with_publication_store(store.clone());
        let (release, waiting) = futures::channel::oneshot::channel::<()>();
        let waiting = Arc::new(std::sync::Mutex::new(Some(waiting)));
        let client = FakeHttpClient::create(move |mut request| {
            let waiting = waiting.clone();
            async move {
                if let Some(value) = preflight(request.uri().path()) {
                    return Ok(response(value));
                }
                assert_eq!(request.method(), http::Method::POST);
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let wait = waiting.lock().unwrap().take().unwrap();
                wait.await.unwrap();
                Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                ))
            }
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        let review = open.review().clone();
        let contexts = open.contexts().clone();
        let target = origin();
        let mut create = Box::pin(service.publish(&review, &contexts, &target, 1, &connection));
        assert!(create.as_mut().now_or_never().is_none());
        service
            .queue_delete(&comparison(), &origin(), 1, "please explain")
            .unwrap();
        open.begin_edit(1);
        open.commit("newer local C");
        open.delete(1);
        assert_eq!(open.current().comments[0].body, "newer local C");
        release.send(()).unwrap();
        let created = futures::executor::block_on(create).unwrap();
        assert_eq!(created.receipt().unwrap().note_id, 51);
        assert_eq!(created.deletion, Some(DeleteStatus::Pending));
        let mut another = origin();
        if let ReviewOrigin::GitLab { iid, .. } = &mut another {
            *iid = 20;
        }
        let mut confirmed = created.clone();
        confirmed.deletion = Some(DeleteStatus::Confirmed);
        store.save(&comparison(), &another, 1, &created).unwrap();
        store.save(&comparison(), &origin(), 1, &confirmed).unwrap();
        open.delete(1);
        assert_eq!(open.current().comments[0].body, "newer local C");
    }

    #[test]
    fn website_deleted_keeps_local_work_until_explicit_new_publication_and_denial_is_not_absence() {
        for denied in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let previous = store.load(&comparison(), &origin(), 1).unwrap().unwrap();
            let client = FakeHttpClient::create(move |request| async move {
                assert_eq!(request.method(), http::Method::GET);
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.uri().query().is_some() && !denied {
                    return Ok(response(serde_json::json!([])));
                }
                Ok(http::Response::builder()
                    .status(if denied { 403 } else { 404 })
                    .body(AsyncBody::empty())
                    .unwrap())
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            service
                .queue_delete(&comparison(), &origin(), 1, "please explain")
                .unwrap();
            let record = futures::executor::block_on(service.delete(
                &comparison(),
                &origin(),
                1,
                &connection,
                false,
            ))
            .unwrap();
            assert_eq!(record.website_deleted, !denied);
            if denied {
                assert!(matches!(record.deletion, Some(DeleteStatus::Failed(_))));
            }
            open.delete(1);
            assert_eq!(open.current().comments.len(), 1);
            if !denied {
                let client = FakeHttpClient::create(|mut request| async move {
                    if let Some(value) = preflight(request.uri().path()) {
                        return Ok(response(value));
                    }
                    assert_eq!(request.method(), http::Method::POST);
                    let mut bytes = Vec::new();
                    request.body_mut().read_to_end(&mut bytes).await.unwrap();
                    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    Ok(response(
                        serde_json::json!({"id":"discussion-2","notes":[{"id":52,"body":payload["body"],"author":{"id":42},"position":payload["position"]}]}),
                    ))
                });
                let record =
                    futures::executor::block_on(PublicationService::new(client, store).publish(
                        open.review(),
                        open.contexts(),
                        &origin(),
                        1,
                        &connection,
                    ))
                    .unwrap();
                assert_eq!(record.receipt().unwrap().note_id, 52);
                assert_ne!(record.operation_id, previous.operation_id);
                assert!(record.deletion.is_none());
                assert!(!record.website_deleted);
            }
        }
    }

    #[test]
    fn failed_unknown_and_interrupted_deletions_survive_reopen_without_losing_identity_or_writing()
    {
        use futures::FutureExt;
        for status in [403, 500, 0] {
            let directory = tempfile::tempdir().unwrap();
            let (mut open, store) = published(directory.path());
            let client = FakeHttpClient::create(move |request| async move {
                if request.uri().path().ends_with("/user") {
                    return Ok(response(serde_json::json!({"id":42})));
                }
                if request.method() == http::Method::GET {
                    return Ok(response(
                        serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                    ));
                }
                assert_eq!(request.method(), http::Method::DELETE);
                if status == 0 {
                    futures::future::pending().await
                } else {
                    Ok(http::Response::builder()
                        .status(status)
                        .body(AsyncBody::empty())
                        .unwrap())
                }
            });
            let service = PublicationService::new(client, store.clone());
            let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
            service
                .queue_delete(&comparison(), &origin(), 1, "please explain")
                .unwrap();
            if status == 0 {
                assert!(
                    service
                        .delete(&comparison(), &origin(), 1, &connection, false)
                        .now_or_never()
                        .is_none()
                );
            } else {
                futures::executor::block_on(service.delete(
                    &comparison(),
                    &origin(),
                    1,
                    &connection,
                    false,
                ))
                .unwrap();
            }
            let reader = PublicationService::new(website_client("please explain"), store.clone());
            let reopened = reader
                .restore_interrupted(&comparison(), &origin(), 1)
                .unwrap();
            if status == 403 {
                assert!(matches!(reopened.deletion, Some(DeleteStatus::Failed(_))));
            } else {
                assert!(matches!(reopened.deletion, Some(DeleteStatus::Unknown(_))));
            }
            let read =
                futures::executor::block_on(reader.read(&comparison(), &origin(), 1, &connection))
                    .unwrap();
            let refreshed = reader
                .reconcile(&comparison(), &origin(), 1, "please explain", false, read)
                .unwrap();
            assert_eq!(refreshed.record.deletion, reopened.deletion);
            assert_eq!(refreshed.record.receipt().unwrap().note_id, 51);
            open.delete(1);
            assert_eq!(open.current().comments.len(), 1);
        }
    }

    #[test]
    fn deleting_a_website_changed_body_requires_reconfirmation_and_retains_local_work() {
        let directory = tempfile::tempdir().unwrap();
        let (open, store) = published(directory.path());
        let service = PublicationService::new(website_client("website B"), store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        service
            .queue_delete(&comparison(), &origin(), 1, "please explain")
            .unwrap();
        let conflict = futures::executor::block_on(service.delete(
            &comparison(),
            &origin(),
            1,
            &connection,
            false,
        ))
        .unwrap();
        assert_eq!(
            conflict.deletion,
            Some(DeleteStatus::Conflict {
                website_body: "website B".into()
            })
        );
        let remote =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        let refreshed = service
            .reconcile(&comparison(), &origin(), 1, "please explain", false, remote)
            .unwrap();
        assert!(refreshed.adopt_body.is_none());
        assert_eq!(open.current().comments[0].body, "please explain");
        let changed_again = PublicationService::new(website_client("website C"), store.clone());
        let conflict = futures::executor::block_on(changed_again.delete(
            &comparison(),
            &origin(),
            1,
            &connection,
            true,
        ))
        .unwrap();
        assert_eq!(
            conflict.deletion,
            Some(DeleteStatus::Conflict {
                website_body: "website C".into()
            })
        );
        let client = FakeHttpClient::create(|request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.method() == http::Method::GET {
                return Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"website C","author":{"id":42}}]}),
                ));
            }
            assert_eq!(request.method(), http::Method::DELETE);
            Ok(http::Response::builder()
                .status(204)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let confirmed = futures::executor::block_on(PublicationService::new(client, store).delete(
            &comparison(),
            &origin(),
            1,
            &connection,
            true,
        ))
        .unwrap();
        assert_eq!(confirmed.deletion, Some(DeleteStatus::Confirmed));
    }

    #[test]
    fn deleting_a_known_publication_removes_same_native_note_before_local_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let client = FakeHttpClient::create(|request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.method() == http::Method::GET {
                return Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42}}]}),
                ));
            }
            assert_eq!(request.method(), http::Method::DELETE);
            assert_eq!(
                request.uri().path(),
                "/api/v4/projects/team%2Frepo/merge_requests/10/discussions/discussion-1/notes/51"
            );
            Ok(http::Response::builder()
                .status(204)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let service = PublicationService::new(client, store.clone());
        service
            .queue_delete(&comparison(), &origin(), 1, "please explain")
            .unwrap();
        open.delete(1);
        assert_eq!(open.current().comments.len(), 1);
        let record = futures::executor::block_on(service.delete(
            &comparison(),
            &origin(),
            1,
            &Connection::new("https://gitlab.example.com".into(), "secret".into()),
            false,
        ))
        .unwrap();
        assert_eq!(record.deletion, Some(DeleteStatus::Confirmed));
        open.delete(1);
        assert!(open.current().comments.is_empty());
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        );
        assert!(reopened.current().comments.is_empty());
    }

    #[test]
    fn reopening_an_interrupted_update_retains_sent_body_without_implicit_confirmation_or_write() {
        use futures::FutureExt;
        let directory = tempfile::tempdir().unwrap();
        let (mut open, store) = published(directory.path());
        let client = FakeHttpClient::create(|request| async move {
            if request.uri().path().ends_with("/user") {
                return Ok(response(serde_json::json!({"id":42})));
            }
            if request.method() == http::Method::GET {
                return Ok(response(
                    serde_json::json!({"id":"discussion-1","notes":[{"id":51,"body":"please explain","author":{"id":42},"position":{"new_line":2}}]}),
                ));
            }
            futures::future::pending().await
        });
        let service = PublicationService::new(client, store.clone());
        let connection = Connection::new("https://gitlab.example.com".into(), "secret".into());
        open.begin_edit(1);
        open.commit("sent B");
        service
            .queue_edit(&comparison(), &origin(), 1, "sent B")
            .unwrap();
        assert!(
            service
                .update(&comparison(), &origin(), 1, &connection, false)
                .now_or_never()
                .is_none()
        );
        drop(open);
        let reopened = OpenReview::reopen(
            comparison(),
            "a.rs",
            origin(),
            ReviewStore::new(directory.path().join("local")),
        );
        let service = PublicationService::new(website_client("sent B"), store.clone());
        let interrupted = service
            .restore_interrupted(&comparison(), &origin(), 1)
            .unwrap();
        assert!(
            matches!(interrupted.edit.as_ref().unwrap().status,EditStatus::Unknown {ref sent_body,..} if sent_body=="sent B")
        );
        let remote =
            futures::executor::block_on(service.read(&comparison(), &origin(), 1, &connection))
                .unwrap();
        let outcome = service
            .reconcile(
                &comparison(),
                &origin(),
                1,
                &reopened.current().comments[0].body,
                false,
                remote,
            )
            .unwrap();
        assert!(outcome.adopt_body.is_none());
        assert_eq!(outcome.record.edit, interrupted.edit);
        assert_eq!(
            outcome.record.receipt().unwrap().confirmed_body,
            "please explain"
        );
        assert_eq!(reopened.current().comments[0].body, "sent B");
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
        assert!(matches!(record.state, PublicationState::Unsupported(_)));
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
