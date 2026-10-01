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
}
#[derive(Deserialize)]
struct VersionDiff {
    old_path: String,
    new_path: String,
    diff: String,
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
    pub async fn prepare(
        &self,
        origin: &ReviewOrigin,
        comment: &DraftComment,
        context: Option<&CommentContext>,
        connection: &Connection,
    ) -> Result<GitLabDiffPosition, String> {
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
            return Err("File comments are not yet supported for publication".into());
        };
        if span.count != 1 || span.start == 0 {
            return Err(
                "This release currently publishes single-line comments; range support is pending"
                    .into(),
            );
        }
        let context =
            context.ok_or("Original file context unavailable; comment retained locally")?;
        let text = match side {
            Side::Preimage => &context.preimage,
            Side::Postimage => &context.postimage,
        };
        if text.lines().count() < span.start as usize {
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
        let version: Version =
            serde_json::from_slice(&bytes).map_err(|_| "GitLab returned invalid diff data")?;
        if version.base_commit_sha != *base_sha
            || version.head_commit_sha != *head_sha
            || version.start_commit_sha != matched.start_commit_sha
            || version
                .start_commit_sha
                .parse::<crate::domain::Oid>()
                .is_err()
        {
            return Err("GitLab returned a different reviewed version".into());
        }
        let files:Vec<_>=version.diffs.iter().filter(|file|match side {Side::Preimage=>&file.old_path,Side::Postimage=>&file.new_path}==path).collect();
        if files.len() != 1 {
            return Err(
                "Selected file cannot be uniquely located in the reviewed GitLab version".into(),
            );
        }
        let file = files[0];
        if file.too_large || file.collapsed || file.diff.is_empty() {
            return Err("GitLab omitted this file's diff; position cannot yet be verified".into());
        }
        let (old_line, new_line) = single_line(&file.diff, *side, span.start)?;
        if let (Some(old), Some(new)) = (old_line, new_line) {
            let preimage = old
                .checked_sub(1)
                .and_then(|index| context.preimage.lines().nth(index as usize));
            let postimage = new
                .checked_sub(1)
                .and_then(|index| context.postimage.lines().nth(index as usize));
            if preimage.is_none() || preimage != postimage {
                return Err("Unchanged position does not match both captured file versions".into());
            }
        }
        Ok(GitLabDiffPosition {
            position_type: "text".into(),
            base_sha: base_sha.clone(),
            start_sha: version.start_commit_sha,
            head_sha: head_sha.clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            old_line,
            new_line,
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
                if let Some(next) = next.filter(|next| !next.is_empty()) {
                    let next = next
                        .parse::<u64>()
                        .map_err(|_| "Invalid GitLab pagination")?;
                    if next <= page {
                        return Err("Invalid GitLab pagination".into());
                    }
                    page = next;
                } else if full {
                    page += 1;
                } else {
                    return Ok(None);
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

/// Use GitLab's raw patch, independently of local ignore-whitespace presentation.
fn single_line(
    patch: &str,
    side: Side,
    selected: u32,
) -> Result<(Option<u32>, Option<u32>), String> {
    let headers = regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap();
    let (mut old, mut new) = (1u32, 1u32);
    let mut has_header = false;
    for line in patch.lines() {
        if let Some(header) = headers.captures(line) {
            let old_start = header[1]
                .parse::<u32>()
                .map_err(|_| "Invalid GitLab diff header")?;
            let new_start = header[2]
                .parse::<u32>()
                .map_err(|_| "Invalid GitLab diff header")?;
            let (cursor, start) = match side {
                Side::Preimage => (old, old_start),
                Side::Postimage => (new, new_start),
            };
            if selected >= cursor && selected < start {
                let delta = selected - cursor;
                return Ok((Some(old + delta), Some(new + delta)));
            }
            old = old_start;
            new = new_start;
            has_header = true;
            continue;
        }
        if !has_header {
            continue;
        }
        let kind = line.as_bytes().first().copied();
        let (old_hit, new_hit) = match kind {
            Some(b'+') => (None, Some(new)),
            Some(b'-') => (Some(old), None),
            Some(b' ') => (Some(old), Some(new)),
            Some(b'\\') => continue,
            _ => return Err("Unrecognized GitLab diff data".into()),
        };
        if match side {
            Side::Preimage => old_hit,
            Side::Postimage => new_hit,
        } == Some(selected)
        {
            return Ok((old_hit, new_hit));
        }
        if old_hit.is_some() {
            old = old.checked_add(1).ok_or("Invalid GitLab line number")?;
        }
        if new_hit.is_some() {
            new = new.checked_add(1).ok_or("Invalid GitLab line number")?;
        }
    }
    if !has_header {
        return Err("GitLab diff has no text position data".into());
    }
    let cursor = match side {
        Side::Preimage => old,
        Side::Postimage => new,
    };
    if selected >= cursor {
        let delta = selected - cursor;
        return Ok((old.checked_add(delta), new.checked_add(delta)));
    }
    Err("Selected line cannot be mapped to the reviewed GitLab diff".into())
}
