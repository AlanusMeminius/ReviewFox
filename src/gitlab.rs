//! GitLab API helpers (Phase A: verify; Phase B: project; Phase C: MR list; Phase D: MR detail).

use futures::AsyncReadExt;
use gpui_http_client::{AsyncBody, HttpClient, http};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

use crate::git::RemoteUrlError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    Network(String),
    Unauthorized,
    Other { status: u16, detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    Ok { username: String },
    Err(VerifyError),
}

#[derive(Deserialize)]
struct GitLabUser {
    username: String,
}

pub fn user_api_url(base_url: &str) -> String {
    format!("{}/api/v4/user", base_url.trim_end_matches('/'))
}

pub async fn verify_pat(http: Arc<dyn HttpClient>, base_url: &str, pat: &str) -> VerifyResult {
    if pat.trim().is_empty() {
        return VerifyResult::Err(VerifyError::Unauthorized);
    }

    let url = user_api_url(base_url);
    let request = match http::Request::builder()
        .method(http::Method::GET)
        .uri(&url)
        .header("PRIVATE-TOKEN", pat)
        .body(AsyncBody::empty())
    {
        Ok(r) => r,
        Err(e) => {
            return VerifyResult::Err(VerifyError::Network(e.to_string()));
        }
    };

    let mut response = match http.send(request).await {
        Ok(r) => r,
        Err(e) => return VerifyResult::Err(VerifyError::Network(e.to_string())),
    };

    let status = response.status();
    let mut body = Vec::new();
    if let Err(e) = response.body_mut().read_to_end(&mut body).await {
        return VerifyResult::Err(VerifyError::Network(e.to_string()));
    }

    if status.as_u16() == 401 {
        return VerifyResult::Err(VerifyError::Unauthorized);
    }
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body).into_owned();
        return VerifyResult::Err(VerifyError::Other {
            status: status.as_u16(),
            detail,
        });
    }

    match serde_json::from_slice::<GitLabUser>(&body) {
        Ok(user) => VerifyResult::Ok {
            username: user.username,
        },
        Err(e) => VerifyResult::Err(VerifyError::Other {
            status: status.as_u16(),
            detail: e.to_string(),
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitLabProjectIdentity {
    pub path_with_namespace: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseRemoteError {
    Empty,
    Unrecognized(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedGitRemote {
    pub host: String,
    pub path_with_namespace: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveProjectError {
    Remote(RemoteUrlError),
    HostMismatch {
        settings_host: String,
        remote_host: String,
    },
    Network(String),
    Unauthorized,
    NotFound,
    Other {
        status: u16,
        detail: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveProjectResult {
    Ok(GitLabProjectIdentity),
    Err(ResolveProjectError),
}

/// The Settings field that fixes an error; `Open Settings` focuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTarget {
    GitLabToken,
    GitLabUrl,
}

impl ResolveProjectError {
    /// Where in Settings the user fixes this (Base URL host, token), if anywhere.
    pub fn settings_fix(&self) -> Option<SettingsTarget> {
        match self {
            Self::HostMismatch { .. } => Some(SettingsTarget::GitLabUrl),
            Self::Unauthorized => Some(SettingsTarget::GitLabToken),
            _ => None,
        }
    }
}

pub fn host_from_base_url(base_url: &str) -> Option<String> {
    url::Url::parse(base_url.trim())
        .ok()
        .and_then(|u| u.host_str().map(normalize_host))
}

pub fn normalize_host(host: &str) -> String {
    host.trim().to_lowercase()
}

pub fn hosts_match(settings_base_url: &str, remote_host: &str) -> bool {
    match host_from_base_url(settings_base_url) {
        Some(settings) => settings == normalize_host(remote_host),
        None => false,
    }
}

fn strip_git_suffix(path: &str) -> String {
    let p = path.trim().trim_start_matches('/').trim_end_matches('/');
    p.strip_suffix(".git").unwrap_or(p).to_string()
}

pub fn parse_git_remote_url(raw: &str) -> Result<ParsedGitRemote, ParseRemoteError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ParseRemoteError::Empty);
    }

    if trimmed.contains("://") {
        return parse_as_url(trimmed);
    }

    // SCP-style: [user@]host:path
    if let Some(colon) = trimmed.rfind(':') {
        let host_part = trimmed[..colon].trim();
        let path_part = trimmed[colon + 1..].trim();
        if !host_part.is_empty() && !path_part.is_empty() {
            let host = host_part
                .rsplit('@')
                .next()
                .ok_or_else(|| ParseRemoteError::Unrecognized(trimmed.to_string()))?;
            return Ok(ParsedGitRemote {
                host: normalize_host(host),
                path_with_namespace: strip_git_suffix(path_part),
            });
        }
    }

    Err(ParseRemoteError::Unrecognized(trimmed.to_string()))
}

fn parse_as_url(raw: &str) -> Result<ParsedGitRemote, ParseRemoteError> {
    let url = url::Url::parse(raw).map_err(|_| ParseRemoteError::Unrecognized(raw.to_string()))?;
    let Some(host) = url.host_str() else {
        return Err(ParseRemoteError::Unrecognized(raw.to_string()));
    };
    let path = url.path();
    let path_with_namespace = strip_git_suffix(path);
    if path_with_namespace.is_empty() {
        return Err(ParseRemoteError::Unrecognized(raw.to_string()));
    }
    Ok(ParsedGitRemote {
        host: normalize_host(host),
        path_with_namespace,
    })
}

pub fn pick_remote_for_settings(
    remotes: &[(String, String)],
    settings_base_url: &str,
) -> Result<(String, String, ParsedGitRemote), ResolveProjectError> {
    let settings_host = host_from_base_url(settings_base_url).unwrap_or_else(|| "?".into());
    let mut parsed_hosts = Vec::new();
    let mut matches: Vec<(String, String, ParsedGitRemote)> = Vec::new();

    for (name, url) in remotes {
        match parse_git_remote_url(url) {
            Ok(parsed) => {
                parsed_hosts.push(format!("{name}→{}", parsed.host));
                if hosts_match(settings_base_url, &parsed.host) {
                    matches.push((name.clone(), url.clone(), parsed));
                }
            }
            Err(_) => parsed_hosts.push(format!("{name}→(unparsed)")),
        }
    }

    if let Some(origin_match) = matches.iter().find(|(name, _, _)| name == "origin") {
        return Ok(origin_match.clone());
    }
    if let Some(first) = matches.into_iter().next() {
        return Ok(first);
    }

    let remote_host = if parsed_hosts.is_empty() {
        "(none)".into()
    } else {
        parsed_hosts.join(", ")
    };
    Err(ResolveProjectError::HostMismatch {
        settings_host,
        remote_host,
    })
}

pub fn project_api_url(base_url: &str, path_with_namespace: &str) -> String {
    let encoded = path_with_namespace.replace('/', "%2F");
    format!(
        "{}/api/v4/projects/{}",
        base_url.trim_end_matches('/'),
        encoded
    )
}

#[derive(Deserialize)]
struct GitLabProject {
    path_with_namespace: String,
}

pub async fn resolve_project(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    repo_path: &Path,
) -> ResolveProjectResult {
    let remotes = match crate::git::list_remote_urls(repo_path) {
        Ok(v) => v,
        Err(e) => return ResolveProjectResult::Err(ResolveProjectError::Remote(e)),
    };

    let (_name, _url, parsed) = match pick_remote_for_settings(&remotes, base_url) {
        Ok(v) => v,
        Err(e) => return ResolveProjectResult::Err(e),
    };

    if pat.trim().is_empty() {
        return ResolveProjectResult::Ok(GitLabProjectIdentity {
            path_with_namespace: parsed.path_with_namespace,
        });
    }

    match fetch_project(http, base_url, pat, &parsed.path_with_namespace).await {
        Ok(identity) => ResolveProjectResult::Ok(identity),
        Err(e) => ResolveProjectResult::Err(e),
    }
}

/// Max open MRs fetched per request (first page only in Phase C).
pub const OPEN_MR_PER_PAGE: u32 = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequestSummary {
    pub iid: u64,
    pub title: String,
    pub source_branch: String,
    pub target_branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListMergeRequestsError {
    MissingPat,
    Network(String),
    Unauthorized,
    NotFound,
    Other { status: u16, detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListMergeRequestsResult {
    Ok(Vec<MergeRequestSummary>),
    Err(ListMergeRequestsError),
}

impl ListMergeRequestsError {
    /// Where in Settings the user fixes this (missing or rejected token), if anywhere.
    pub fn settings_fix(&self) -> Option<SettingsTarget> {
        matches!(self, Self::MissingPat | Self::Unauthorized).then_some(SettingsTarget::GitLabToken)
    }
}

pub fn open_merge_requests_api_url(base_url: &str, path_with_namespace: &str) -> String {
    format!(
        "{}/merge_requests?state=opened&per_page={OPEN_MR_PER_PAGE}",
        project_api_url(base_url, path_with_namespace)
    )
}

#[derive(Deserialize)]
struct GitLabMergeRequest {
    iid: u64,
    title: String,
    source_branch: String,
    target_branch: String,
}

pub fn format_resolve_project_error(e: &ResolveProjectError) -> String {
    match e {
        ResolveProjectError::Remote(crate::git::RemoteUrlError::NoRemotes) => {
            "This repository has no git remotes.".into()
        }
        ResolveProjectError::Remote(crate::git::RemoteUrlError::Open(msg)) => msg.clone(),
        ResolveProjectError::HostMismatch {
            settings_host,
            remote_host,
        } => format!(
            "No git remote host matches Settings Base URL host `{settings_host}` (remotes: {remote_host})."
        ),
        ResolveProjectError::Network(msg) => format!("Network error: {msg}"),
        ResolveProjectError::Unauthorized => "Unauthorized (401). Check token and scopes.".into(),
        ResolveProjectError::NotFound => {
            "Project not found on GitLab (404). Check path and token access.".into()
        }
        ResolveProjectError::Other { status, detail } => format!("HTTP {status}: {detail}"),
    }
}

pub fn format_list_merge_requests_error(e: &ListMergeRequestsError) -> String {
    match e {
        ListMergeRequestsError::MissingPat => {
            "Add a personal access token in Settings to list merge requests.".into()
        }
        ListMergeRequestsError::Network(msg) => format!("Network error: {msg}"),
        ListMergeRequestsError::Unauthorized => {
            "Unauthorized (401). Check token and scopes.".into()
        }
        ListMergeRequestsError::NotFound => {
            "Project not found on GitLab (404). Check path and token access.".into()
        }
        ListMergeRequestsError::Other { status, detail } => format!("HTTP {status}: {detail}"),
    }
}

pub async fn list_open_merge_requests(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    path_with_namespace: &str,
) -> ListMergeRequestsResult {
    if pat.trim().is_empty() {
        return ListMergeRequestsResult::Err(ListMergeRequestsError::MissingPat);
    }

    let url = open_merge_requests_api_url(base_url, path_with_namespace);
    let request = match http::Request::builder()
        .method(http::Method::GET)
        .uri(&url)
        .header("PRIVATE-TOKEN", pat)
        .body(AsyncBody::empty())
    {
        Ok(r) => r,
        Err(e) => {
            return ListMergeRequestsResult::Err(ListMergeRequestsError::Network(e.to_string()));
        }
    };

    let mut response = match http.send(request).await {
        Ok(r) => r,
        Err(e) => {
            return ListMergeRequestsResult::Err(ListMergeRequestsError::Network(e.to_string()));
        }
    };

    let status = response.status().as_u16();
    let mut body = Vec::new();
    if let Err(e) = response.body_mut().read_to_end(&mut body).await {
        return ListMergeRequestsResult::Err(ListMergeRequestsError::Network(e.to_string()));
    }

    if status == 401 {
        return ListMergeRequestsResult::Err(ListMergeRequestsError::Unauthorized);
    }
    if status == 404 {
        return ListMergeRequestsResult::Err(ListMergeRequestsError::NotFound);
    }
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&body).into_owned();
        return ListMergeRequestsResult::Err(ListMergeRequestsError::Other { status, detail });
    }

    match serde_json::from_slice::<Vec<GitLabMergeRequest>>(&body) {
        Ok(rows) => ListMergeRequestsResult::Ok(
            rows.into_iter()
                .map(|mr| MergeRequestSummary {
                    iid: mr.iid,
                    title: mr.title,
                    source_branch: mr.source_branch,
                    target_branch: mr.target_branch,
                })
                .collect(),
        ),
        Err(e) => ListMergeRequestsResult::Err(ListMergeRequestsError::Other {
            status,
            detail: e.to_string(),
        }),
    }
}

/// Comparison OID pair from GitLab (ADR-0006); consumed by Phase E/F.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MrDiffRefs {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequestCheckState {
    pub pipeline_status: Option<String>,
    pub approved: Option<bool>,
    pub approvals_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequestDetail {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub author_username: String,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub merge_status: Option<String>,
    pub diff_refs: MrDiffRefs,
    pub check_state: MergeRequestCheckState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchMergeRequestError {
    MissingPat,
    Network(String),
    Unauthorized,
    NotFound,
    MissingDiffRefs,
    Other { status: u16, detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchMergeRequestResult {
    Ok(MergeRequestDetail),
    Err(FetchMergeRequestError),
}

impl FetchMergeRequestError {
    /// Where in Settings the user fixes this (missing or rejected token), if anywhere.
    pub fn settings_fix(&self) -> Option<SettingsTarget> {
        matches!(self, Self::MissingPat | Self::Unauthorized).then_some(SettingsTarget::GitLabToken)
    }
}

pub fn merge_request_api_url(base_url: &str, path_with_namespace: &str, iid: u64) -> String {
    format!(
        "{}/merge_requests/{iid}",
        project_api_url(base_url, path_with_namespace)
    )
}

pub fn merge_request_approvals_api_url(
    base_url: &str,
    path_with_namespace: &str,
    iid: u64,
) -> String {
    format!(
        "{}/approvals",
        merge_request_api_url(base_url, path_with_namespace, iid)
    )
}

pub fn format_fetch_merge_request_error(e: &FetchMergeRequestError) -> String {
    match e {
        FetchMergeRequestError::MissingPat => {
            "Add a personal access token in Settings to load merge request detail.".into()
        }
        FetchMergeRequestError::Network(msg) => format!("Network error: {msg}"),
        FetchMergeRequestError::Unauthorized => {
            "Unauthorized (401). Check token and scopes.".into()
        }
        FetchMergeRequestError::NotFound => "Merge request not found on GitLab (404).".into(),
        FetchMergeRequestError::MissingDiffRefs => {
            "GitLab did not return diff_refs for this merge request.".into()
        }
        FetchMergeRequestError::Other { status, detail } => format!("HTTP {status}: {detail}"),
    }
}

#[derive(Deserialize)]
struct GitLabDiffRefs {
    base_sha: String,
    head_sha: String,
    #[serde(default)]
    start_sha: Option<String>,
}

#[derive(Deserialize)]
struct GitLabAuthor {
    username: String,
}

#[derive(Deserialize)]
struct GitLabPipeline {
    status: String,
}

#[derive(Deserialize)]
struct GitLabMergeRequestDetail {
    iid: u64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    state: String,
    source_branch: String,
    target_branch: String,
    author: GitLabAuthor,
    diff_refs: Option<GitLabDiffRefs>,
    head_pipeline: Option<GitLabPipeline>,
    #[serde(default)]
    merge_status: Option<String>,
}

#[derive(Deserialize)]
struct GitLabApprovals {
    approved: bool,
    approvals_required: u32,
    approvals_left: u32,
}

fn parse_diff_refs(raw: Option<GitLabDiffRefs>) -> Result<MrDiffRefs, FetchMergeRequestError> {
    let Some(refs) = raw else {
        return Err(FetchMergeRequestError::MissingDiffRefs);
    };
    if refs.base_sha.trim().is_empty() || refs.head_sha.trim().is_empty() {
        return Err(FetchMergeRequestError::MissingDiffRefs);
    }
    Ok(MrDiffRefs {
        base_sha: refs.base_sha,
        head_sha: refs.head_sha,
        start_sha: refs.start_sha.filter(|s| !s.trim().is_empty()),
    })
}

fn detail_from_payload(
    mr: GitLabMergeRequestDetail,
    diff_refs: MrDiffRefs,
    check_state: MergeRequestCheckState,
) -> MergeRequestDetail {
    MergeRequestDetail {
        iid: mr.iid,
        title: mr.title,
        description: mr.description.filter(|d| !d.trim().is_empty()),
        author_username: mr.author.username,
        state: mr.state,
        source_branch: mr.source_branch,
        target_branch: mr.target_branch,
        merge_status: mr.merge_status.filter(|s| !s.trim().is_empty()),
        diff_refs,
        check_state: MergeRequestCheckState {
            pipeline_status: mr
                .head_pipeline
                .map(|p| p.status)
                .filter(|s| !s.trim().is_empty()),
            ..check_state
        },
    }
}

async fn gitlab_get(
    http: Arc<dyn HttpClient>,
    pat: &str,
    url: &str,
) -> Result<(u16, Vec<u8>), FetchMergeRequestError> {
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(url)
        .header("PRIVATE-TOKEN", pat)
        .body(AsyncBody::empty())
        .map_err(|e| FetchMergeRequestError::Network(e.to_string()))?;

    let mut response = http
        .send(request)
        .await
        .map_err(|e| FetchMergeRequestError::Network(e.to_string()))?;

    let status = response.status().as_u16();
    let mut body = Vec::new();
    if let Err(e) = response.body_mut().read_to_end(&mut body).await {
        return Err(FetchMergeRequestError::Network(e.to_string()));
    }
    Ok((status, body))
}

async fn fetch_merge_request_approvals(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    path_with_namespace: &str,
    iid: u64,
) -> MergeRequestCheckState {
    let url = merge_request_approvals_api_url(base_url, path_with_namespace, iid);
    let Ok((status, body)) = gitlab_get(http, pat, &url).await else {
        return MergeRequestCheckState {
            pipeline_status: None,
            approved: None,
            approvals_label: None,
        };
    };
    if status != 200 {
        return MergeRequestCheckState {
            pipeline_status: None,
            approved: None,
            approvals_label: None,
        };
    }
    match serde_json::from_slice::<GitLabApprovals>(&body) {
        Ok(a) => {
            let label = if a.approvals_required == 0 {
                if a.approved {
                    Some("approved".into())
                } else {
                    None
                }
            } else {
                let have = a.approvals_required.saturating_sub(a.approvals_left);
                Some(format!("{have}/{} approvals", a.approvals_required))
            };
            MergeRequestCheckState {
                pipeline_status: None,
                approved: Some(a.approved),
                approvals_label: label,
            }
        }
        Err(_) => MergeRequestCheckState {
            pipeline_status: None,
            approved: None,
            approvals_label: None,
        },
    }
}

pub async fn fetch_merge_request(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    path_with_namespace: &str,
    iid: u64,
) -> FetchMergeRequestResult {
    if pat.trim().is_empty() {
        return FetchMergeRequestResult::Err(FetchMergeRequestError::MissingPat);
    }

    let url = merge_request_api_url(base_url, path_with_namespace, iid);
    let (status, body) = match gitlab_get(http.clone(), pat, &url).await {
        Ok(v) => v,
        Err(e) => return FetchMergeRequestResult::Err(e),
    };

    if status == 401 {
        return FetchMergeRequestResult::Err(FetchMergeRequestError::Unauthorized);
    }
    if status == 404 {
        return FetchMergeRequestResult::Err(FetchMergeRequestError::NotFound);
    }
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&body).into_owned();
        return FetchMergeRequestResult::Err(FetchMergeRequestError::Other { status, detail });
    }

    let mut payload = match serde_json::from_slice::<GitLabMergeRequestDetail>(&body) {
        Ok(mr) => mr,
        Err(e) => {
            return FetchMergeRequestResult::Err(FetchMergeRequestError::Other {
                status,
                detail: e.to_string(),
            });
        }
    };

    let diff_refs = match parse_diff_refs(payload.diff_refs.take()) {
        Ok(refs) => refs,
        Err(e) => return FetchMergeRequestResult::Err(e),
    };

    let check_state =
        fetch_merge_request_approvals(http, base_url, pat, path_with_namespace, iid).await;

    FetchMergeRequestResult::Ok(detail_from_payload(payload, diff_refs, check_state))
}

/// True when the repo has at least one remote whose host matches Settings Base URL.
pub fn repo_matches_settings_host(repo_path: &Path, settings_base_url: &str) -> bool {
    let Ok(remotes) = crate::git::list_remote_urls(repo_path) else {
        return false;
    };
    pick_remote_for_settings(&remotes, settings_base_url).is_ok()
}

/// Matching remote `(name, url)` for fetch, if any.
pub fn matching_remote_for_settings(
    repo_path: &Path,
    settings_base_url: &str,
) -> Result<(String, String), ResolveProjectError> {
    let remotes = crate::git::list_remote_urls(repo_path).map_err(ResolveProjectError::Remote)?;
    let (name, url, _) = pick_remote_for_settings(&remotes, settings_base_url)?;
    Ok((name, url))
}

/// Max commits fetched for an MR (first page; matches GitLab web for typical MRs).
pub const MR_COMMITS_PER_PAGE: u32 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequestCommit {
    pub id: String,
    pub title: String,
    pub author_name: String,
    pub authored_date: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListMergeRequestCommitsError {
    MissingPat,
    Network(String),
    Unauthorized,
    NotFound,
    Other { status: u16, detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListMergeRequestCommitsResult {
    Ok(Vec<MergeRequestCommit>),
    Err(ListMergeRequestCommitsError),
}

impl ListMergeRequestCommitsError {
    /// Where in Settings the user fixes this (missing or rejected token), if anywhere.
    pub fn settings_fix(&self) -> Option<SettingsTarget> {
        matches!(self, Self::MissingPat | Self::Unauthorized).then_some(SettingsTarget::GitLabToken)
    }
}

pub fn merge_request_commits_api_url(
    base_url: &str,
    path_with_namespace: &str,
    iid: u64,
) -> String {
    format!(
        "{}/commits?per_page={MR_COMMITS_PER_PAGE}",
        merge_request_api_url(base_url, path_with_namespace, iid)
    )
}

pub fn format_list_merge_request_commits_error(e: &ListMergeRequestCommitsError) -> String {
    match e {
        ListMergeRequestCommitsError::MissingPat => {
            "Add a personal access token in Settings to load merge request commits.".into()
        }
        ListMergeRequestCommitsError::Network(msg) => format!("Network error: {msg}"),
        ListMergeRequestCommitsError::Unauthorized => {
            "Unauthorized (401). Check token and scopes.".into()
        }
        ListMergeRequestCommitsError::NotFound => {
            "Merge request commits not found on GitLab (404).".into()
        }
        ListMergeRequestCommitsError::Other { status, detail } => {
            format!("HTTP {status}: {detail}")
        }
    }
}

#[derive(Deserialize)]
struct GitLabMrCommit {
    id: String,
    title: String,
    author_name: String,
    #[serde(default)]
    authored_date: String,
}

pub async fn list_merge_request_commits(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    path_with_namespace: &str,
    iid: u64,
) -> ListMergeRequestCommitsResult {
    if pat.trim().is_empty() {
        return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::MissingPat);
    }

    let url = merge_request_commits_api_url(base_url, path_with_namespace, iid);
    let request = match http::Request::builder()
        .method(http::Method::GET)
        .uri(&url)
        .header("PRIVATE-TOKEN", pat)
        .body(AsyncBody::empty())
    {
        Ok(r) => r,
        Err(e) => {
            return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Network(
                e.to_string(),
            ));
        }
    };

    let mut response = match http.send(request).await {
        Ok(r) => r,
        Err(e) => {
            return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Network(
                e.to_string(),
            ));
        }
    };

    let status = response.status().as_u16();
    let mut body = Vec::new();
    if let Err(e) = response.body_mut().read_to_end(&mut body).await {
        return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Network(
            e.to_string(),
        ));
    }

    if status == 401 {
        return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Unauthorized);
    }
    if status == 404 {
        return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::NotFound);
    }
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&body).into_owned();
        return ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Other {
            status,
            detail,
        });
    }

    match serde_json::from_slice::<Vec<GitLabMrCommit>>(&body) {
        Ok(rows) => {
            // GitLab returns oldest→newest; Branch Browser fold expects newest-first.
            let mut commits: Vec<MergeRequestCommit> = rows
                .into_iter()
                .map(|c| MergeRequestCommit {
                    id: c.id,
                    title: c.title,
                    author_name: c.author_name,
                    authored_date: c.authored_date,
                })
                .collect();
            commits.reverse();
            ListMergeRequestCommitsResult::Ok(commits)
        }
        Err(e) => ListMergeRequestCommitsResult::Err(ListMergeRequestCommitsError::Other {
            status,
            detail: e.to_string(),
        }),
    }
}

async fn fetch_project(
    http: Arc<dyn HttpClient>,
    base_url: &str,
    pat: &str,
    path_with_namespace: &str,
) -> Result<GitLabProjectIdentity, ResolveProjectError> {
    let url = project_api_url(base_url, path_with_namespace);
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(&url)
        .header("PRIVATE-TOKEN", pat)
        .body(AsyncBody::empty())
        .map_err(|e| ResolveProjectError::Network(e.to_string()))?;

    let mut response = http
        .send(request)
        .await
        .map_err(|e| ResolveProjectError::Network(e.to_string()))?;

    let status = response.status().as_u16();
    let mut body = Vec::new();
    if let Err(e) = response.body_mut().read_to_end(&mut body).await {
        return Err(ResolveProjectError::Network(e.to_string()));
    }

    if status == 401 {
        return Err(ResolveProjectError::Unauthorized);
    }
    if status == 404 {
        return Err(ResolveProjectError::NotFound);
    }
    if !(200..300).contains(&status) {
        let detail = String::from_utf8_lossy(&body).into_owned();
        return Err(ResolveProjectError::Other { status, detail });
    }

    match serde_json::from_slice::<GitLabProject>(&body) {
        Ok(project) => Ok(GitLabProjectIdentity {
            path_with_namespace: project.path_with_namespace,
        }),
        Err(e) => Err(ResolveProjectError::Other {
            status,
            detail: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_http_client::StatusCode;

    #[test]
    fn user_api_url_joins_base() {
        assert_eq!(
            user_api_url("https://gitlab.com"),
            "https://gitlab.com/api/v4/user"
        );
    }

    #[test]
    fn verify_ok_parses_username() {
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            assert_eq!(req.uri().to_string(), "https://gitlab.com/api/v4/user");
            assert_eq!(req.headers().get("PRIVATE-TOKEN").unwrap(), "secret");
            Ok(http::Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from(r#"{"username":"alice"}"#))
                .unwrap())
        });
        let result =
            futures::executor::block_on(verify_pat(client, "https://gitlab.com", "secret"));
        assert_eq!(
            result,
            VerifyResult::Ok {
                username: "alice".into()
            }
        );
    }

    #[test]
    fn verify_401() {
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            Ok(http::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let result = futures::executor::block_on(verify_pat(client, "https://gitlab.com", "bad"));
        assert_eq!(result, VerifyResult::Err(VerifyError::Unauthorized));
    }

    #[test]
    fn parse_https_remote() {
        let p = parse_git_remote_url("https://gitlab.com/group/sub/repo.git").unwrap();
        assert_eq!(p.host, "gitlab.com");
        assert_eq!(p.path_with_namespace, "group/sub/repo");
    }

    #[test]
    fn parse_https_with_user() {
        let p = parse_git_remote_url("https://oauth2:token@gitlab.example.com/foo/bar").unwrap();
        assert_eq!(p.host, "gitlab.example.com");
        assert_eq!(p.path_with_namespace, "foo/bar");
    }

    #[test]
    fn parse_ssh_url() {
        let p = parse_git_remote_url("ssh://git@gitlab.com/group/repo.git").unwrap();
        assert_eq!(p.host, "gitlab.com");
        assert_eq!(p.path_with_namespace, "group/repo");
    }

    #[test]
    fn parse_scp_style() {
        let p = parse_git_remote_url("git@gitlab.com:group/sub/repo.git").unwrap();
        assert_eq!(p.host, "gitlab.com");
        assert_eq!(p.path_with_namespace, "group/sub/repo");
    }

    #[test]
    fn hosts_match_case_insensitive() {
        assert!(hosts_match("https://GitLab.COM", "gitlab.com"));
        assert!(hosts_match(
            "https://gitlab.example.com/",
            "gitlab.example.com"
        ));
        assert!(!hosts_match("https://gitlab.com", "github.com"));
    }

    #[test]
    fn host_mismatch_error() {
        let remotes = vec![("origin".into(), "git@github.com:org/repo.git".into())];
        let err = pick_remote_for_settings(&remotes, "https://gitlab.com").unwrap_err();
        assert_eq!(
            err,
            ResolveProjectError::HostMismatch {
                settings_host: "gitlab.com".into(),
                remote_host: "origin→github.com".into(),
            }
        );
    }

    #[test]
    fn pick_remote_skips_github_origin_for_gitlab_settings() {
        let remotes = vec![
            ("origin".into(), "git@github.com:org/mirror.git".into()),
            (
                "gitlab".into(),
                "git@gitlab.lan.example.com:group/repo.git".into(),
            ),
        ];
        let (name, _url, parsed) =
            pick_remote_for_settings(&remotes, "https://gitlab.lan.example.com").unwrap();
        assert_eq!(name, "gitlab");
        assert_eq!(parsed.path_with_namespace, "group/repo");
    }

    #[test]
    fn pick_remote_prefers_matching_origin() {
        let remotes = vec![
            ("gitlab".into(), "https://gitlab.com/other/repo.git".into()),
            ("origin".into(), "https://gitlab.com/main/repo.git".into()),
        ];
        let (name, _, parsed) = pick_remote_for_settings(&remotes, "https://gitlab.com").unwrap();
        assert_eq!(name, "origin");
        assert_eq!(parsed.path_with_namespace, "main/repo");
    }

    #[test]
    fn resolve_project_uses_non_origin_matching_remote() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/org/mirror.git",
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "remote",
                "add",
                "gitlab",
                "https://gitlab.lan.example.com/acme/widget.git",
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            panic!("should not call HTTP without PAT");
        });
        let result = futures::executor::block_on(resolve_project(
            client,
            "https://gitlab.lan.example.com",
            "",
            dir.path(),
        ));
        assert_eq!(
            result,
            ResolveProjectResult::Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
            })
        );
    }

    #[test]
    fn project_api_url_encodes_path() {
        assert_eq!(
            project_api_url("https://gitlab.com", "a/b/c"),
            "https://gitlab.com/api/v4/projects/a%2Fb%2Fc"
        );
    }

    #[test]
    fn resolve_project_without_pat_skips_api() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://gitlab.com/my/group.git"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            panic!("should not call HTTP without PAT");
        });
        let result = futures::executor::block_on(resolve_project(
            client,
            "https://gitlab.com",
            "",
            dir.path(),
        ));
        assert_eq!(
            result,
            ResolveProjectResult::Ok(GitLabProjectIdentity {
                path_with_namespace: "my/group".into(),
            })
        );
    }

    #[test]
    fn resolve_project_confirms_via_api() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "git@gitlab.com:acme/widget.git"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            assert_eq!(
                req.uri().to_string(),
                "https://gitlab.com/api/v4/projects/acme%2Fwidget"
            );
            Ok(http::Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from(r#"{"path_with_namespace":"acme/widget"}"#))
                .unwrap())
        });
        let result = futures::executor::block_on(resolve_project(
            client,
            "https://gitlab.com",
            "tok",
            dir.path(),
        ));
        assert_eq!(
            result,
            ResolveProjectResult::Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
            })
        );
    }

    #[test]
    fn open_merge_requests_url_includes_query() {
        assert_eq!(
            open_merge_requests_api_url("https://gitlab.com", "g/p"),
            "https://gitlab.com/api/v4/projects/g%2Fp/merge_requests?state=opened&per_page=50"
        );
    }

    #[test]
    fn list_open_mrs_parses_page() {
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            assert_eq!(
                req.uri().to_string(),
                "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests?state=opened&per_page=50"
            );
            Ok(http::Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from(
                    r#"[
                        {"iid":7,"title":"Fix bug","source_branch":"feat","target_branch":"main"}
                    ]"#,
                ))
                .unwrap())
        });
        let result = futures::executor::block_on(list_open_merge_requests(
            client,
            "https://gitlab.com",
            "tok",
            "acme/widget",
        ));
        assert_eq!(
            result,
            ListMergeRequestsResult::Ok(vec![MergeRequestSummary {
                iid: 7,
                title: "Fix bug".into(),
                source_branch: "feat".into(),
                target_branch: "main".into(),
            }])
        );
    }

    #[test]
    fn list_open_mrs_missing_pat() {
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            panic!("no HTTP without PAT");
        });
        let result = futures::executor::block_on(list_open_merge_requests(
            client,
            "https://gitlab.com",
            "  ",
            "acme/widget",
        ));
        assert_eq!(
            result,
            ListMergeRequestsResult::Err(ListMergeRequestsError::MissingPat)
        );
    }

    #[test]
    fn merge_request_api_url_includes_iid() {
        assert_eq!(
            merge_request_api_url("https://gitlab.com", "g/p", 12),
            "https://gitlab.com/api/v4/projects/g%2Fp/merge_requests/12"
        );
    }

    #[test]
    fn fetch_merge_request_parses_diff_refs() {
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            let uri = req.uri().to_string();
            if uri.ends_with("/merge_requests/3") {
                Ok(http::Response::builder()
                    .status(StatusCode::OK)
                    .body(AsyncBody::from(
                        r#"{
                            "iid": 3,
                            "title": "Add feature",
                            "description": "Details here",
                            "state": "opened",
                            "source_branch": "feat",
                            "target_branch": "main",
                            "author": {"username": "bob"},
                            "merge_status": "can_be_merged",
                            "diff_refs": {
                                "base_sha": "aaa0000000000000000000000000000000000000",
                                "head_sha": "bbb1111111111111111111111111111111111111",
                                "start_sha": "ccc2222222222222222222222222222222222222"
                            },
                            "head_pipeline": {"status": "success"}
                        }"#,
                    ))
                    .unwrap())
            } else if uri.ends_with("/approvals") {
                Ok(http::Response::builder()
                    .status(StatusCode::OK)
                    .body(AsyncBody::from(
                        r#"{"approved": true, "approvals_required": 2, "approvals_left": 0}"#,
                    ))
                    .unwrap())
            } else {
                panic!("unexpected URI: {uri}");
            }
        });
        let result = futures::executor::block_on(fetch_merge_request(
            client,
            "https://gitlab.com",
            "tok",
            "acme/widget",
            3,
        ));
        assert_eq!(
            result,
            FetchMergeRequestResult::Ok(MergeRequestDetail {
                iid: 3,
                title: "Add feature".into(),
                description: Some("Details here".into()),
                author_username: "bob".into(),
                state: "opened".into(),
                source_branch: "feat".into(),
                target_branch: "main".into(),
                merge_status: Some("can_be_merged".into()),
                diff_refs: MrDiffRefs {
                    base_sha: "aaa0000000000000000000000000000000000000".into(),
                    head_sha: "bbb1111111111111111111111111111111111111".into(),
                    start_sha: Some("ccc2222222222222222222222222222222222222".into()),
                },
                check_state: MergeRequestCheckState {
                    pipeline_status: Some("success".into()),
                    approved: Some(true),
                    approvals_label: Some("2/2 approvals".into()),
                },
            })
        );
    }

    #[test]
    fn fetch_merge_request_missing_diff_refs() {
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            assert!(req.uri().to_string().ends_with("/merge_requests/1"));
            Ok(http::Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from(
                    r#"{
                        "iid": 1,
                        "title": "X",
                        "state": "opened",
                        "source_branch": "a",
                        "target_branch": "b",
                        "author": {"username": "u"},
                        "diff_refs": null
                    }"#,
                ))
                .unwrap())
        });
        let result = futures::executor::block_on(fetch_merge_request(
            client,
            "https://gitlab.com",
            "tok",
            "g/p",
            1,
        ));
        assert_eq!(
            result,
            FetchMergeRequestResult::Err(FetchMergeRequestError::MissingDiffRefs)
        );
    }

    #[test]
    fn fetch_merge_request_404() {
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            Ok(http::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let result = futures::executor::block_on(fetch_merge_request(
            client,
            "https://gitlab.com",
            "tok",
            "g/p",
            99,
        ));
        assert_eq!(
            result,
            FetchMergeRequestResult::Err(FetchMergeRequestError::NotFound)
        );
    }

    #[test]
    fn resolve_project_404() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://gitlab.com/missing/proj.git",
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            Ok(http::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(AsyncBody::empty())
                .unwrap())
        });
        let result = futures::executor::block_on(resolve_project(
            client,
            "https://gitlab.com",
            "tok",
            dir.path(),
        ));
        assert_eq!(
            result,
            ResolveProjectResult::Err(ResolveProjectError::NotFound)
        );
    }

    #[test]
    fn settings_fix_targets_token_or_url_only() {
        use SettingsTarget::{GitLabToken, GitLabUrl};

        assert_eq!(
            ResolveProjectError::Unauthorized.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(
            ResolveProjectError::HostMismatch {
                settings_host: "gitlab.com".into(),
                remote_host: "github.com".into(),
            }
            .settings_fix(),
            Some(GitLabUrl)
        );
        assert_eq!(
            ResolveProjectError::Network("timeout".into()).settings_fix(),
            None
        );
        assert_eq!(ResolveProjectError::NotFound.settings_fix(), None);

        assert_eq!(
            ListMergeRequestsError::MissingPat.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(
            ListMergeRequestsError::Unauthorized.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(ListMergeRequestsError::NotFound.settings_fix(), None);

        assert_eq!(
            FetchMergeRequestError::MissingPat.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(
            FetchMergeRequestError::Unauthorized.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(FetchMergeRequestError::MissingDiffRefs.settings_fix(), None);

        assert_eq!(
            ListMergeRequestCommitsError::MissingPat.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(
            ListMergeRequestCommitsError::Unauthorized.settings_fix(),
            Some(GitLabToken)
        );
        assert_eq!(
            ListMergeRequestCommitsError::Other {
                status: 500,
                detail: "boom".into(),
            }
            .settings_fix(),
            None
        );
    }

    #[test]
    fn repo_matches_settings_host_detects_gitlab_remote() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/o/r.git"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(!repo_matches_settings_host(
            dir.path(),
            "https://gitlab.lan.example.com"
        ));
        std::process::Command::new("git")
            .args([
                "remote",
                "add",
                "gitlab",
                "https://gitlab.lan.example.com/g/p.git",
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(repo_matches_settings_host(
            dir.path(),
            "https://gitlab.lan.example.com"
        ));
    }

    #[test]
    fn list_mr_commits_reverses_to_newest_first() {
        let client = gpui_http_client::FakeHttpClient::create(|_| async move {
            Ok(http::Response::builder()
                .status(StatusCode::OK)
                .body(AsyncBody::from(
                    r#"[
                      {"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","title":"oldest","author_name":"a","authored_date":"2020-01-01T00:00:00Z"},
                      {"id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","title":"newest","author_name":"b","authored_date":"2020-01-02T00:00:00Z"}
                    ]"#,
                ))
                .unwrap())
        });
        let result = futures::executor::block_on(list_merge_request_commits(
            client,
            "https://gitlab.com",
            "tok",
            "g/p",
            1,
        ));
        match result {
            ListMergeRequestCommitsResult::Ok(commits) => {
                assert_eq!(commits.len(), 2);
                assert_eq!(commits[0].title, "newest");
                assert_eq!(commits[1].title, "oldest");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }
}
