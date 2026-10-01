//! GitLab's versioned positions and native discussion-create contract.
use crate::{
    domain::{Anchor, DraftComment, Side},
    export::CommentContext,
    publication::{
        Connection, PublicationReceipt, PublicationRecord, PublicationState, ReviewOrigin,
    },
};
use futures::{AsyncReadExt, FutureExt};
use gpui_http_client::{AsyncBody, HttpClient, RedirectPolicy, http};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

pub enum PreparationError {
    Failed(String),
    Unsupported(String),
}
impl From<String> for PreparationError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}
impl From<&str> for PreparationError {
    fn from(message: &str) -> Self {
        Self::Failed(message.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLabDiffPosition {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<GitLabLineRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLabLineRange {
    pub start: GitLabRangeEndpoint,
    pub end: GitLabRangeEndpoint,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLabRangeEndpoint {
    pub line_code: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

#[derive(Clone)]
pub struct GitLabPublication {
    http: Arc<dyn HttpClient>,
}
#[derive(Deserialize)]
struct Author {
    id: u64,
}
#[derive(Deserialize)]
struct Version {
    id: u64,
    base_commit_sha: String,
    head_commit_sha: String,
    start_commit_sha: String,
    #[serde(default)]
    diffs: Vec<VersionDiff>,
    #[serde(default)]
    state: Option<String>,
}
#[derive(Deserialize)]
struct VersionDiff {
    old_path: String,
    new_path: String,
    #[serde(default)]
    diff: String,
    #[serde(default)]
    renamed_file: bool,
    #[serde(default)]
    too_large: bool,
    #[serde(default)]
    collapsed: bool,
}
#[derive(Deserialize)]
struct Discussion {
    id: String,
    notes: Vec<Note>,
}
#[derive(Deserialize)]
struct Note {
    id: u64,
    body: String,
    author: Author,
    #[serde(default)]
    position: serde_json::Value,
    #[serde(default)]
    resolved: Option<bool>,
    #[serde(default)]
    outdated: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct RemoteNote {
    pub body: String,
    pub position: serde_json::Value,
    pub resolved: Option<bool>,
    /// None when the server's API does not report this capability.
    pub outdated: Option<bool>,
}

pub enum UpdateOutcome {
    Confirmed(RemoteNote),
    Failed(String),
    Unknown(String),
}
pub enum DeleteOutcome {
    Confirmed,
    Failed(String),
    Unknown(String),
}

impl GitLabPublication {
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self { http }
    }
    async fn request(
        &self,
        method: http::Method,
        url: String,
        pat: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(u16, Vec<u8>, Option<String>), &'static str> {
        let request = http::Request::builder()
            .method(method)
            .uri(url)
            .header("PRIVATE-TOKEN", pat)
            .header("Content-Type", "application/json")
            .extension(RedirectPolicy::NoFollow)
            .body(
                body.map(|body| AsyncBody::from(body.to_string()))
                    .unwrap_or_else(AsyncBody::empty),
            )
            .map_err(|_| "Invalid request configuration")?;
        let response = async {
            let mut response = self
                .http
                .send(request)
                .await
                .map_err(|_| "Network request failed")?;
            let status = response.status().as_u16();
            let next = response
                .headers()
                .get("X-Next-Page")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            response
                .body_mut()
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "Response was interrupted")?;
            Ok((status, bytes, next))
        }
        .boxed();
        let timeout = gpui::Timer::after(Duration::from_secs(30)).boxed();
        match futures::future::select(response, timeout).await {
            futures::future::Either::Left((result, _)) => result,
            futures::future::Either::Right(_) => Err("Request timed out"),
        }
    }
    fn endpoint(origin: &ReviewOrigin) -> Result<String, String> {
        let ReviewOrigin::GitLab {
            base_url,
            project,
            iid,
            ..
        } = origin
        else {
            return Err("This Review has no GitLab target".into());
        };
        Ok(crate::gitlab::merge_request_api_url(
            base_url, project, *iid,
        ))
    }
    async fn get(
        &self,
        url: String,
        connection: &Connection,
    ) -> Result<(Vec<u8>, Option<String>), String> {
        let (status, bytes, next) = self
            .request(http::Method::GET, url, &connection.pat, None)
            .await
            .map_err(str::to_owned)?;
        if !(200..300).contains(&status) {
            return Err(status_message(status));
        }
        Ok((bytes, next))
    }
    pub async fn author(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
    ) -> Result<u64, String> {
        let ReviewOrigin::GitLab { base_url, .. } = origin else {
            return Err("No GitLab target".into());
        };
        let (bytes, _) = self
            .get(crate::gitlab::user_api_url(base_url), connection)
            .await?;
        let author: Author = serde_json::from_slice(&bytes)
            .map_err(|_| "GitLab returned an invalid account response")?;
        if author.id == 0 {
            return Err("GitLab account identity unavailable".into());
        }
        Ok(author.id)
    }
    async fn historical_blob(
        &self,
        origin: &ReviewOrigin,
        path: &str,
        sha: &str,
        connection: &Connection,
    ) -> Result<Arc<str>, String> {
        let ReviewOrigin::GitLab {
            base_url, project, ..
        } = origin
        else {
            return Err("No GitLab target".into());
        };
        let path = url::form_urlencoded::byte_serialize(path.as_bytes()).collect::<String>();
        let mut url = url::Url::parse(&format!(
            "{}/repository/files/{path}/raw",
            crate::gitlab::project_api_url(base_url, project)
        ))
        .map_err(|_| "Invalid historical file URL")?;
        url.query_pairs_mut().append_pair("ref", sha);
        let (bytes, _) = self
            .get(url.to_string(), connection)
            .await
            .map_err(|error| format!("Cannot read reviewed rename verification blob: {error}"))?;
        String::from_utf8(bytes)
            .map(Arc::from)
            .map_err(|_| "Reviewed rename blob is not UTF-8 text; local work retained".into())
    }
    pub async fn prepare(
        &self,
        origin: &ReviewOrigin,
        comment: &DraftComment,
        context: Option<&CommentContext>,
        connection: &Connection,
    ) -> Result<GitLabDiffPosition, PreparationError> {
        let ReviewOrigin::GitLab {
            base_sha,
            head_sha,
            start_sha,
            ..
        } = origin
        else {
            return Err("No GitLab target".into());
        };
        let Anchor::Line {
            path, side, span, ..
        } = &comment.anchor
        else {
            return Err(PreparationError::Unsupported(
                "File comments are not yet supported for publication".into(),
            ));
        };
        if span.count == 0 || span.start == 0 {
            return Err("Selected range is empty or invalid".into());
        }
        let context =
            context.ok_or("Original file context unavailable; comment retained locally")?;
        let text = match side {
            Side::Preimage => &context.preimage,
            Side::Postimage => &context.postimage,
        };
        let end = span
            .start
            .checked_add(span.count - 1)
            .ok_or("Selected range exceeds supported line numbers")?;
        if text.lines().count() < end as usize {
            return Err("Selected line is outside its captured file".into());
        }
        let endpoint = Self::endpoint(origin)?;
        let mut page = "1".to_string();
        let mut matching = Vec::new();
        loop {
            let (bytes, next) = self
                .get(
                    format!("{endpoint}/versions?per_page=100&page={page}"),
                    connection,
                )
                .await?;
            let versions: Vec<Version> = serde_json::from_slice(&bytes)
                .map_err(|_| "GitLab returned invalid diff version metadata")?;
            matching.extend(versions.into_iter().filter(|version| {
                version.base_commit_sha == *base_sha
                    && version.head_commit_sha == *head_sha
                    && start_sha
                        .as_ref()
                        .is_none_or(|start| *start == version.start_commit_sha)
            }));
            match next.filter(|page| !page.is_empty()) {
                Some(next)
                    if next
                        .parse::<u64>()
                        .ok()
                        .is_some_and(|value| value > page.parse::<u64>().unwrap_or(0)) =>
                {
                    page = next
                }
                Some(_) => return Err("Invalid GitLab version pagination".into()),
                None => break,
            }
        }
        if matching.len() != 1 {
            return Err("Cannot identify a unique reviewed MR version; no latest-version substitution was made".into());
        }
        let matched = &matching[0];
        let (bytes, _) = self
            .get(format!("{endpoint}/versions/{}", matched.id), connection)
            .await?;
        let mut version: Version =
            serde_json::from_slice(&bytes).map_err(|_| "GitLab returned invalid diff data")?;
        if version.id != matched.id
            || version.base_commit_sha != *base_sha
            || version.head_commit_sha != *head_sha
            || version.start_commit_sha != matched.start_commit_sha
            || version
                .start_commit_sha
                .parse::<crate::domain::Oid>()
                .is_err()
        {
            return Err("GitLab returned a different reviewed version".into());
        }
        let locate_file = |version: &Version| -> Result<usize, String> {
            let primary:Vec<_>=version.diffs.iter().enumerate().filter(|(_,file)|match side {Side::Preimage=>&file.old_path,Side::Postimage=>&file.new_path}==path).map(|(index,_)|index).collect();
            let candidates = if primary.is_empty() {
                version
                    .diffs
                    .iter()
                    .enumerate()
                    .filter(|(_, file)| {
                        file.renamed_file && (file.old_path == *path || file.new_path == *path)
                    })
                    .map(|(index, _)| index)
                    .collect()
            } else {
                primary
            };
            if candidates.len() != 1 {
                return Err(match version.state.as_deref() {
                    Some("overflow"|"without_files"|"overflow_diff_files_limit"|"overflow_diff_lines_limit") => "GitLab diff version exceeded its file/line limits; selected historical file unavailable, local work retained".into(),
                    _ => "Selected file cannot be uniquely located in the reviewed GitLab version".into(),
                });
            }
            Ok(candidates[0])
        };
        let mut index = locate_file(&version)?;
        if version.diffs[index].collapsed
            && !version.diffs[index].too_large
            && version.diffs[index].diff.is_empty()
        {
            // Supported version-pinned representation request. Never use the
            // current MR raw_diffs endpoint to fill a historical omission.
            let (bytes, _) = self
                .get(
                    format!("{endpoint}/versions/{}?unidiff=true", matched.id),
                    connection,
                )
                .await?;
            let expanded: Version = serde_json::from_slice(&bytes)
                .map_err(|_| "GitLab returned invalid unified historical diff data")?;
            if expanded.id != version.id
                || expanded.base_commit_sha != version.base_commit_sha
                || expanded.start_commit_sha != version.start_commit_sha
                || expanded.head_commit_sha != version.head_commit_sha
            {
                return Err("GitLab returned a different version during unified diff retrieval; local work retained".into());
            }
            index = locate_file(&expanded)?;
            version = expanded;
        }
        let file = &version.diffs[index];
        if file.too_large {
            return Err("GitLab too_large limit excludes this file's diff; keep the comment and Export locally".into());
        }
        if file.collapsed && file.diff.is_empty() {
            return Err("GitLab still omitted this collapsed historical file after unified retrieval; view that version on GitLab or keep it local".into());
        }
        if file.diff.is_empty() && file.renamed_file {
            return Err(
                "Rename-only file has no GitLab text section; keep the line comment locally".into(),
            );
        }
        if file.diff.is_empty() {
            return Err("GitLab returned no text diff for this historical file; local comment and Export retained".into());
        }
        let file_path = if file.new_path.is_empty() {
            &file.old_path
        } else {
            &file.new_path
        };
        // Rename detection can show this as an Add or Delete locally. Recover
        // only the missing verification side from the reviewed commits; never
        // change the authored context used by the editor or Export.
        let mut verification = context.clone();
        if file.renamed_file {
            if verification.preimage.is_empty() {
                verification.preimage = self
                    .historical_blob(origin, &file.old_path, base_sha, connection)
                    .await?;
            }
            if verification.postimage.is_empty() {
                verification.postimage = self
                    .historical_blob(origin, &file.new_path, head_sha, connection)
                    .await?;
            }
        }
        let (old_line, new_line, line_range) =
            selected_positions(&file.diff, *side, span.start, end, file_path, &verification)
                .map_err(PreparationError::Unsupported)?;
        Ok(GitLabDiffPosition {
            position_type: "text".into(),
            base_sha: base_sha.clone(),
            start_sha: version.start_commit_sha,
            head_sha: head_sha.clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            old_line,
            new_line,
            line_range,
        })
    }
    fn known_note(
        discussion: Discussion,
        record: &PublicationRecord,
    ) -> Result<Option<RemoteNote>, String> {
        let receipt = record.receipt().ok_or("Remote identity unavailable")?;
        if discussion.id != receipt.discussion_id {
            return Err("GitLab returned another discussion".into());
        }
        let Some(note) = discussion
            .notes
            .into_iter()
            .find(|note| note.id == receipt.note_id)
        else {
            return Ok(None);
        };
        if Some(note.author.id) != record.author_id {
            return Err("Remote comment author no longer matches".into());
        }
        Ok(Some(RemoteNote {
            body: record.visible_body(&note.body),
            position: note.position,
            resolved: note.resolved,
            outdated: note.outdated,
        }))
    }
    /// Markers are editable content, so every matching native note is a candidate.
    pub async fn find_marked(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> Result<Vec<PublicationReceipt>, String> {
        let endpoint = Self::endpoint(origin)?;
        let author = record
            .author_id
            .ok_or("Original account identity unavailable; marker recovery cannot be verified")?;
        let mut page = 1u64;
        let mut candidates = Vec::new();
        loop {
            let (bytes, next) = self
                .get(
                    format!("{endpoint}/discussions?per_page=100&page={page}"),
                    connection,
                )
                .await?;
            let discussions: Vec<Discussion> =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid GitLab discussion response")?;
            let full = discussions.len() == 100;
            for discussion in discussions {
                if discussion.id.is_empty() {
                    return Err("Invalid native discussion identity".into());
                }
                for note in discussion.notes {
                    if note.author.id == author && note.body.contains(&record.marker()) {
                        if note.id == 0 {
                            return Err("Invalid native comment identity".into());
                        }
                        candidates.push(PublicationReceipt {
                            discussion_id: discussion.id.clone(),
                            note_id: note.id,
                            confirmed_body: record.visible_body(&note.body),
                            remote_position: note.position,
                            resolved: note.resolved,
                            outdated: note.outdated,
                        });
                    }
                }
            }
            match next_discussion_page(page, next, full)? {
                Some(next) => page = next,
                None => break,
            }
        }
        candidates.sort_by_key(|candidate| candidate.note_id);
        candidates.dedup_by(|a, b| a.note_id == b.note_id && a.discussion_id == b.discussion_id);
        Ok(candidates)
    }

    pub async fn read_known(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> Result<Option<RemoteNote>, String> {
        let endpoint = Self::endpoint(origin)?;
        let receipt = record.receipt().ok_or("Remote identity unavailable")?;
        let discussion_id = url::form_urlencoded::byte_serialize(receipt.discussion_id.as_bytes())
            .collect::<String>();
        let (status, bytes, _) = self
            .request(
                http::Method::GET,
                format!("{endpoint}/discussions/{discussion_id}"),
                &connection.pat,
                None,
            )
            .await
            .map_err(str::to_owned)?;
        if status == 404 {
            // A 404 alone can mean denied access. Verify absence in a successful paginated list.
            let mut page = 1;
            loop {
                let (bytes, next) = self
                    .get(
                        format!("{endpoint}/discussions?per_page=100&page={page}"),
                        connection,
                    )
                    .await?;
                let discussions: Vec<Discussion> = serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid GitLab discussion response")?;
                let full = discussions.len() == 100;
                if let Some(discussion) = discussions
                    .into_iter()
                    .find(|discussion| discussion.id == receipt.discussion_id)
                {
                    return Self::known_note(discussion, record);
                }
                match next_discussion_page(page, next, full)? {
                    Some(next) => page = next,
                    None => return Ok(None),
                }
            }
        }
        if !(200..300).contains(&status) {
            return Err(status_message(status));
        }
        let discussion: Discussion =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid GitLab discussion response")?;
        Self::known_note(discussion, record)
    }

    // GitLab has no conditional body PUT here. A website edit between the pre-read
    // and this request can still race; checking first does not provide atomicity.
    pub async fn update_known(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> UpdateOutcome {
        let Ok(endpoint) = Self::endpoint(origin) else {
            return UpdateOutcome::Failed("No GitLab target".into());
        };
        let Some(receipt) = record.receipt() else {
            return UpdateOutcome::Failed("Remote identity unavailable".into());
        };
        let discussion_id = url::form_urlencoded::byte_serialize(receipt.discussion_id.as_bytes())
            .collect::<String>();
        let url = format!(
            "{endpoint}/discussions/{discussion_id}/notes/{}",
            receipt.note_id
        );
        match self
            .request(
                http::Method::PUT,
                url,
                &connection.pat,
                Some(serde_json::json!({"body":record.remote_body()})),
            )
            .await
        {
            Err(message) => UpdateOutcome::Unknown(message.into()),
            Ok((status, _, _))
                if matches!(status, 400 | 401 | 403 | 404 | 405 | 409 | 413 | 422 | 429) =>
            {
                UpdateOutcome::Failed(status_message(status))
            }
            Ok((status, bytes, _)) if (200..300).contains(&status) => {
                match serde_json::from_slice::<Note>(&bytes) {
                    Ok(note)
                        if note.id == receipt.note_id
                            && Some(note.author.id) == record.author_id
                            && note.body == record.remote_body() =>
                    {
                        UpdateOutcome::Confirmed(RemoteNote {
                            body: record.body.clone(),
                            position: note.position,
                            resolved: note.resolved,
                            outdated: note.outdated,
                        })
                    }
                    _ => UpdateOutcome::Unknown(
                        "Update response did not confirm the expected body".into(),
                    ),
                }
            }
            Ok((status, _, _)) => {
                UpdateOutcome::Unknown(format!("GitLab HTTP {status}; update may have succeeded"))
            }
        }
    }

    pub async fn delete_known(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> DeleteOutcome {
        let Ok(endpoint) = Self::endpoint(origin) else {
            return DeleteOutcome::Failed("No GitLab target".into());
        };
        let Some(receipt) = record.receipt() else {
            return DeleteOutcome::Failed("Remote identity unavailable".into());
        };
        let discussion_id = url::form_urlencoded::byte_serialize(receipt.discussion_id.as_bytes())
            .collect::<String>();
        let url = format!(
            "{endpoint}/discussions/{discussion_id}/notes/{}",
            receipt.note_id
        );
        match self
            .request(http::Method::DELETE, url, &connection.pat, None)
            .await
        {
            Ok((204, _, _)) => DeleteOutcome::Confirmed,
            Ok((status, _, _))
                if matches!(status, 400 | 401 | 403 | 404 | 405 | 409 | 413 | 422 | 429) =>
            {
                DeleteOutcome::Failed(status_message(status))
            }
            Ok((status, _, _)) => {
                DeleteOutcome::Unknown(format!("GitLab HTTP {status}; deletion may have succeeded"))
            }
            Err(message) => DeleteOutcome::Unknown(message.into()),
        }
    }

    pub async fn create(
        &self,
        origin: &ReviewOrigin,
        connection: &Connection,
        record: &PublicationRecord,
    ) -> PublicationState {
        let Ok(endpoint) = Self::endpoint(origin) else {
            return PublicationState::Failed("No GitLab target".into());
        };
        let body = serde_json::json!({"body":record.remote_body(),"position":record.position});
        match self
            .request(
                http::Method::POST,
                format!("{endpoint}/discussions"),
                &connection.pat,
                Some(body),
            )
            .await
        {
            Err(message) => PublicationState::Unknown(format!(
                "{message}; do not publish again before checking GitLab"
            )),
            Ok((status, _, _))
                if matches!(status, 400 | 401 | 403 | 404 | 405 | 409 | 413 | 422 | 429) =>
            {
                PublicationState::Failed(status_message(status))
            }
            Ok((status, bytes, _)) if (200..300).contains(&status) => {
                let discussion = serde_json::from_slice::<Discussion>(&bytes);
                match discussion {
                    Ok(discussion) if !discussion.id.is_empty() && discussion.notes.len() == 1 => {
                        let note = &discussion.notes[0];
                        if note.id == 0
                            || Some(note.author.id) != record.author_id
                            || note.body != record.remote_body()
                        {
                            return PublicationState::Unknown("Create response did not confirm the expected comment; check GitLab".into());
                        }
                        PublicationState::Published(PublicationReceipt {
                            discussion_id: discussion.id,
                            note_id: note.id,
                            confirmed_body: record.body.clone(),
                            remote_position: note.position.clone(),
                            resolved: note.resolved,
                            outdated: note.outdated,
                        })
                    }
                    _ => PublicationState::Unknown(
                        "GitLab accepted the request but its response could not be confirmed"
                            .into(),
                    ),
                }
            }
            Ok((status, _, _)) => PublicationState::Unknown(format!(
                "GitLab HTTP {status}; creation may have succeeded"
            )),
        }
    }
}
fn next_discussion_page(
    page: u64,
    next: Option<String>,
    full: bool,
) -> Result<Option<u64>, String> {
    if let Some(next) = next.filter(|next| !next.is_empty()) {
        let next = next
            .parse::<u64>()
            .map_err(|_| "Invalid GitLab pagination")?;
        if next <= page {
            return Err("Invalid GitLab pagination".into());
        }
        Ok(Some(next))
    } else if full {
        page.checked_add(1)
            .map(Some)
            .ok_or_else(|| "Invalid GitLab pagination".into())
    } else {
        Ok(None)
    }
}

fn status_message(status: u16) -> String {
    match status {
        401 => "Authentication failed (401). Check the GitLab token".into(),
        403 => {
            "Permission denied (403). Publishing requires api scope and MR write permission".into()
        }
        404 => "MR or reviewed version unavailable (404)".into(),
        400 | 422 => {
            format!("GitLab rejected the comment or position ({status}); local work retained")
        }
        429 => "GitLab rate limit reached (429); retry explicitly later".into(),
        _ => format!("GitLab request failed (HTTP {status})"),
    }
}

/// A raw GitLab section, independent of local Hunk and ViewOptions.
struct RawSection {
    old_start: u32,
    new_start: u32,
    old_end: u32,
    new_end: u32,
    lines: Vec<RawLine>,
}
#[derive(Clone)]
struct RawLine {
    old: u32,
    new: u32,
    kind: RawLineKind,
    text: Option<String>,
    section: Section,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum RawLineKind {
    Added,
    Removed,
    Context,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Raw(usize),
    Gap(usize),
}
impl RawLine {
    fn old_line(&self) -> Option<u32> {
        (self.kind != RawLineKind::Added).then_some(self.old)
    }
    fn new_line(&self) -> Option<u32> {
        (self.kind != RawLineKind::Removed).then_some(self.new)
    }
    fn endpoint(&self, path: &str) -> GitLabRangeEndpoint {
        GitLabRangeEndpoint {
            line_code: format!(
                "{}_{old}_{new}",
                sha1_smol::Sha1::from(path.as_bytes()).digest(),
                old = self.old,
                new = self.new
            ),
            kind: match self.kind {
                RawLineKind::Added => Some("new".into()),
                RawLineKind::Removed => Some("old".into()),
                RawLineKind::Context => None,
            },
            old_line: self.old_line(),
            new_line: self.new_line(),
        }
    }
    fn validate(&self, preimage: &[&str], postimage: &[&str]) -> Result<(), String> {
        let old = self
            .old_line()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| preimage.get(n as usize).copied());
        let new = self
            .new_line()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| postimage.get(n as usize).copied());
        let valid = match self.kind {
            RawLineKind::Removed => old.is_some() && self.text.as_deref() == old,
            RawLineKind::Added => new.is_some() && self.text.as_deref() == new,
            RawLineKind::Context => {
                old.is_some()
                    && old == new
                    && self.text.as_deref().is_none_or(|text| Some(text) == old)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(
                "GitLab position does not match both captured file versions; local work retained"
                    .into(),
            )
        }
    }
}
fn raw_sections(patch: &str) -> Result<Vec<RawSection>, String> {
    let headers = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();
    let mut sections: Vec<RawSection> = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for line in patch.lines() {
        if let Some(header) = headers.captures(line) {
            if sections
                .last()
                .is_some_and(|last| last.old_end != old || last.new_end != new)
            {
                return Err("GitLab raw section is truncated; position cannot be verified".into());
            }
            old = header[1].parse().map_err(|_| "Invalid GitLab old line")?;
            new = header[3].parse().map_err(|_| "Invalid GitLab new line")?;
            let old_count = header
                .get(2)
                .map_or(Ok(1), |value| value.as_str().parse::<u32>())
                .map_err(|_| "Invalid GitLab section length")?;
            let new_count = header
                .get(4)
                .map_or(Ok(1), |value| value.as_str().parse::<u32>())
                .map_err(|_| "Invalid GitLab section length")?;
            if (old_count > 0 && old == 0) || (new_count > 0 && new == 0) {
                return Err("Invalid GitLab section origin".into());
            }
            let old_end = old
                .checked_add(old_count)
                .ok_or("Invalid GitLab section end")?;
            let new_end = new
                .checked_add(new_count)
                .ok_or("Invalid GitLab section end")?;
            if sections
                .last()
                .is_some_and(|last| last.old_end > old || last.new_end > new)
            {
                return Err("GitLab raw sections overlap; position cannot be verified".into());
            }
            sections.push(RawSection {
                old_start: old,
                new_start: new,
                old_end,
                new_end,
                lines: Vec::new(),
            });
            continue;
        }
        if sections.is_empty() || line.starts_with('\\') {
            continue;
        }
        let kind = match line.as_bytes().first() {
            Some(b'+') => RawLineKind::Added,
            Some(b'-') => RawLineKind::Removed,
            Some(b' ') => RawLineKind::Context,
            _ => return Err("Unrecognized GitLab diff data".into()),
        };
        let index = sections.len() - 1;
        sections[index].lines.push(RawLine {
            old,
            new,
            kind,
            text: Some(line[1..].to_owned()),
            section: Section::Raw(index),
        });
        if kind != RawLineKind::Added {
            old = old.checked_add(1).ok_or("Invalid GitLab old line")?;
        }
        if kind != RawLineKind::Removed {
            new = new.checked_add(1).ok_or("Invalid GitLab new line")?;
        }
    }
    if sections.is_empty() {
        return Err("GitLab diff has no text position data".into());
    }
    if sections
        .last()
        .is_some_and(|last| last.old_end != old || last.new_end != new)
    {
        return Err("GitLab raw section is truncated; position cannot be verified".into());
    }
    Ok(sections)
}
fn locate(
    sections: &[RawSection],
    raw_index: &std::collections::HashMap<u32, &RawLine>,
    side: Side,
    selected: u32,
) -> Result<RawLine, String> {
    if let Some(line) = raw_index.get(&selected) {
        return Ok((*line).clone());
    }
    // Each omitted gap has its own offset. The adjacent headers must agree.
    for gap in 0..=sections.len() {
        let previous = gap.checked_sub(1).map(|index| &sections[index]);
        let next = sections.get(gap);
        let delta = previous
            .map(|section| i64::from(section.new_end) - i64::from(section.old_end))
            .unwrap_or_else(|| i64::from(sections[0].new_start) - i64::from(sections[0].old_start));
        if next.is_some_and(|section| {
            i64::from(section.new_start) - i64::from(section.old_start) != delta
        }) {
            continue;
        }
        let (old, new) = match side {
            Side::Preimage => (i64::from(selected), i64::from(selected) + delta),
            Side::Postimage => (i64::from(selected) - delta, i64::from(selected)),
        };
        let (Ok(old), Ok(new)) = (u32::try_from(old), u32::try_from(new)) else {
            continue;
        };
        if old == 0 || new == 0 {
            continue;
        }
        if previous.is_some_and(|section| old < section.old_end || new < section.new_end)
            || next.is_some_and(|section| old >= section.old_start || new >= section.new_start)
        {
            continue;
        }
        return Ok(RawLine {
            old,
            new,
            kind: RawLineKind::Context,
            text: None,
            section: Section::Gap(gap),
        });
    }
    Err("Selected line cannot be mapped uniquely to the reviewed GitLab diff".into())
}
fn selected_positions(
    patch: &str,
    side: Side,
    start: u32,
    end: u32,
    path: &str,
    context: &CommentContext,
) -> Result<(Option<u32>, Option<u32>, Option<GitLabLineRange>), String> {
    let sections = raw_sections(patch)?;
    let raw_index = sections
        .iter()
        .flat_map(|section| section.lines.iter())
        .filter_map(|line| {
            match side {
                Side::Preimage => line.old_line(),
                Side::Postimage => line.new_line(),
            }
            .map(|number| (number, line))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let selected = (start..=end)
        .map(|number| locate(&sections, &raw_index, side, number))
        .collect::<Result<Vec<_>, _>>()?;
    let preimage = context.preimage.lines().collect::<Vec<_>>();
    let postimage = context.postimage.lines().collect::<Vec<_>>();
    for line in &selected {
        line.validate(&preimage, &postimage)?;
    }
    let first = selected.first().ok_or("Selected range is empty")?;
    let last = selected.last().unwrap();
    let range = if start == end {
        None
    } else {
        let raw_ids = selected
            .iter()
            .filter_map(|line| match line.section {
                Section::Raw(index) => Some(index),
                Section::Gap(_) => None,
            })
            .collect::<std::collections::HashSet<_>>();
        if raw_ids.len() > 1 {
            return Err(
                "Range crosses GitLab diff sections; keep it local or select one section".into(),
            );
        }
        if !selected.iter().all(|line| line.section == first.section)
            || matches!(first.section, Section::Gap(_))
        {
            // GitLab LinesUnfolder uses three blob lines around the final position.
            // Only claim one unfolded section if the whole selection is in that
            // generated window; do not assign gaps to a neighboring raw hunk.
            if !matches!(last.section, Section::Gap(_))
                || last.old.saturating_sub(3) > first.old
                || !selected.iter().all(|line| line.section == last.section)
            {
                return Err("Expanded range cannot be verified within GitLab's three-line unfolding window; select a shorter context range or keep it local".into());
            }
        }
        Some(GitLabLineRange {
            start: first.endpoint(path),
            end: last.endpoint(path),
        })
    };
    Ok((last.old_line(), last.new_line(), range))
}
