//! Read-only discovery across registered Repositories. Never installs a Comparison.
use super::*;
use chrono::{DateTime, Days, Local, Months, NaiveDate, TimeZone, Utc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Period {
    Today,
    TwoDays,
    ThreeDays,
    #[default]
    Week,
    Month,
}

impl Period {
    pub const ALL: [Self; 5] = [
        Self::Today,
        Self::TwoDays,
        Self::ThreeDays,
        Self::Week,
        Self::Month,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今天",
            Self::TwoDays => "近两天",
            Self::ThreeDays => "近三天",
            Self::Week => "近一周",
            Self::Month => "近一个月",
        }
    }

    pub fn start_date(self, today: NaiveDate) -> NaiveDate {
        match self {
            Self::Today => today,
            Self::TwoDays => today - Days::new(1),
            Self::ThreeDays => today - Days::new(2),
            Self::Week => today - Days::new(6),
            Self::Month => today.checked_sub_months(Months::new(1)).unwrap_or(today),
        }
    }

    pub fn start(self, now: DateTime<Local>) -> DateTime<Utc> {
        // Some time zones move clocks at midnight. Use the first valid minute of
        // the civil day, and the earlier occurrence on an ambiguous midnight.
        let date = self.start_date(now.date_naive());
        let midnight = date.and_hms_opt(0, 0, 0).unwrap();
        (0..=1440)
            .find_map(|minute| {
                Local
                    .from_local_datetime(&(midnight + chrono::Duration::minutes(minute)))
                    .earliest()
            })
            .unwrap_or(now)
            .with_timezone(&Utc)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct InboxMr {
    pub iid: u64,
    pub title: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    pub author: InboxAuthor,
    pub updated_at: DateTime<Utc>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InboxAuthor {
    pub username: String,
}

impl InboxMr {
    pub fn summary(&self) -> MergeRequestSummary {
        MergeRequestSummary {
            iid: self.iid,
            title: self.title.clone(),
            source_branch: self.source_branch.clone(),
            target_branch: self.target_branch.clone(),
        }
    }

    pub fn status(&self) -> &str {
        if self.draft && self.state == "opened" {
            "Draft"
        } else {
            &self.state
        }
    }
}

/// Preview remains available even when GitLab has not prepared diff_refs yet.
/// Only the explicit MR Entry open resolves git objects / installs a Comparison.
#[derive(Clone, Debug)]
pub struct Preview {
    pub merge_status: Option<String>,
    pub checks: MergeRequestCheckState,
}

pub async fn preview(
    http: Arc<dyn HttpClient>,
    base: &str,
    pat: &str,
    project: &str,
    iid: u64,
) -> Result<Preview, FetchMergeRequestError> {
    if pat.trim().is_empty() {
        return Err(FetchMergeRequestError::MissingPat);
    }
    let (status, body) = gitlab_get(
        http.clone(),
        pat,
        &merge_request_api_url(base, project, iid),
    )
    .await?;
    match status {
        401 | 403 => return Err(FetchMergeRequestError::Unauthorized),
        404 => return Err(FetchMergeRequestError::NotFound),
        200..=299 => {}
        _ => {
            return Err(FetchMergeRequestError::Other {
                status,
                detail: String::from_utf8_lossy(&body).into_owned(),
            });
        }
    }
    let mr: GitLabMergeRequestDetail =
        serde_json::from_slice(&body).map_err(|e| FetchMergeRequestError::Other {
            status,
            detail: e.to_string(),
        })?;
    let mut checks = fetch_merge_request_approvals(http, base, pat, project, iid).await;
    checks.pipeline_status = mr.head_pipeline.map(|p| p.status);
    Ok(Preview {
        merge_status: mr.merge_status,
        checks,
    })
}

pub async fn list(
    http: Arc<dyn HttpClient>,
    base: &str,
    pat: &str,
    project: &str,
    updated_after: DateTime<Utc>,
) -> Result<Vec<InboxMr>, ListMergeRequestsError> {
    if pat.trim().is_empty() {
        return Err(ListMergeRequestsError::MissingPat);
    }
    let mut page = 1u32;
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        let mut url = url::Url::parse(&format!(
            "{}/merge_requests",
            project_api_url(base, project)
        ))
        .map_err(|e| ListMergeRequestsError::Network(e.to_string()))?;
        url.query_pairs_mut().extend_pairs([
            ("scope", "all"),
            ("state", "all"),
            ("order_by", "updated_at"),
            ("sort", "desc"),
            ("per_page", "100"),
            ("page", &page.to_string()),
            ("updated_after", &updated_after.to_rfc3339()),
        ]);
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri(url.as_str())
            .header("PRIVATE-TOKEN", pat)
            .body(AsyncBody::empty())
            .map_err(|e| ListMergeRequestsError::Network(e.to_string()))?;
        let mut response = http
            .send(request)
            .await
            .map_err(|e| ListMergeRequestsError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let next = response
            .headers()
            .get("x-next-page")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut body)
            .await
            .map_err(|e| ListMergeRequestsError::Network(e.to_string()))?;
        match status {
            401 | 403 => return Err(ListMergeRequestsError::Unauthorized),
            404 => return Err(ListMergeRequestsError::NotFound),
            200..=299 => {}
            _ => {
                return Err(ListMergeRequestsError::Other {
                    status,
                    detail: String::from_utf8_lossy(&body).into_owned(),
                });
            }
        }
        let batch: Vec<InboxMr> =
            serde_json::from_slice(&body).map_err(|e| ListMergeRequestsError::Other {
                status,
                detail: e.to_string(),
            })?;
        let count = batch.len();
        rows.extend(
            batch
                .into_iter()
                .filter(|mr| mr.updated_at >= updated_after && seen.insert(mr.iid)),
        );
        match next.as_deref() {
            Some("") => break,
            Some(value) => {
                page = value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > page)
                    .ok_or_else(|| ListMergeRequestsError::Other {
                        status,
                        detail: "Invalid GitLab pagination header".into(),
                    })?;
            }
            None if count < 100 => break,
            None => page += 1,
        }
    }
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.iid.cmp(&b.iid)));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn civil_days_include_today_and_calendar_month_clamps() {
        let date = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        assert_eq!(Period::Today.start_date(date), date);
        assert_eq!(Period::TwoDays.start_date(date).to_string(), "2024-03-30");
        assert_eq!(Period::ThreeDays.start_date(date).to_string(), "2024-03-29");
        assert_eq!(Period::Week.start_date(date).to_string(), "2024-03-25");
        assert_eq!(Period::Month.start_date(date).to_string(), "2024-02-29");
        assert_eq!(
            Period::TwoDays
                .start_date(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
                .to_string(),
            "2025-12-31"
        );
    }

    fn payload(iid: u64) -> serde_json::Value {
        serde_json::json!({"iid":iid,"title":"MR", "source_branch":"feature", "target_branch":"main", "state":"opened", "author":{"username":"reviewer"}, "updated_at":"2026-10-06T00:00:00Z"})
    }

    #[test]
    fn reads_all_pages_and_deduplicates_moving_rows() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let client = gpui_http_client::FakeHttpClient::create(move |req| {
            let n = count.fetch_add(1, Ordering::SeqCst);
            async move {
                let url = url::Url::parse(&req.uri().to_string()).unwrap();
                let query: std::collections::HashMap<_, _> =
                    url.query_pairs().into_owned().collect();
                assert_eq!(query["state"], "all");
                assert_eq!(query["scope"], "all");
                assert_eq!(query["updated_after"], "2026-10-06T00:00:00+00:00");
                assert_eq!(query["page"], (n + 1).to_string());
                assert_eq!(req.headers()["PRIVATE-TOKEN"], "test-token");
                let rows = if n == 0 {
                    (1..=100).map(payload).collect::<Vec<_>>()
                } else {
                    vec![payload(100), payload(101)]
                };
                Ok(http::Response::builder()
                    .status(200)
                    .header("x-next-page", if n == 0 { "2" } else { "" })
                    .body(AsyncBody::from(serde_json::to_vec(&rows).unwrap()))
                    .unwrap())
            }
        });
        let rows = futures::executor::block_on(list(
            client,
            "https://gitlab.example",
            "test-token",
            "team/repo",
            "2026-10-06T00:00:00Z".parse().unwrap(),
        ))
        .unwrap();
        assert_eq!(rows.len(), 101);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn missing_pat_never_calls_network() {
        let client =
            gpui_http_client::FakeHttpClient::create(|_| async { panic!("network called") });
        assert_eq!(
            futures::executor::block_on(list(
                client,
                "https://gitlab.example",
                "",
                "team/repo",
                Utc::now()
            ))
            .unwrap_err(),
            ListMergeRequestsError::MissingPat
        );
    }

    #[test]
    fn preview_does_not_require_diff_refs() {
        let client = gpui_http_client::FakeHttpClient::create(|req| async move {
            let body = if req.uri().path().ends_with("/approvals") {
                serde_json::json!({"approved":true,"approvals_required":1,"approvals_left":0})
            } else {
                let mut p = payload(3);
                p["head_pipeline"] = serde_json::json!({"status":"success"});
                p
            };
            Ok(http::Response::builder()
                .status(200)
                .body(AsyncBody::from(serde_json::to_vec(&body).unwrap()))
                .unwrap())
        });
        let result = futures::executor::block_on(preview(
            client,
            "https://gitlab.example",
            "test-token",
            "team/repo",
            3,
        ))
        .unwrap();
        assert_eq!(result.checks.pipeline_status.as_deref(), Some("success"));
        assert_eq!(result.checks.approved, Some(true));
    }
}
