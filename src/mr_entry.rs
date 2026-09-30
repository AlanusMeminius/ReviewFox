//! In-process MR Entry resolution.

use std::path::Path;
use std::sync::Arc;

use crate::git::{self, CommitInfo};
use crate::gitlab::{self, GitLabProjectIdentity, MergeRequestCommit, MergeRequestDetail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub detail: MergeRequestDetail,
    pub project: String,
    pub commit_infos: Vec<CommitInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub settings_target: Option<gitlab::SettingsTarget>,
}

pub trait Forge {
    async fn resolve(
        &self,
        repo_path: &Path,
        base_url: &str,
        pat: &str,
    ) -> Result<GitLabProjectIdentity, Failure>;

    async fn fetch_merge_request(
        &self,
        base_url: &str,
        pat: &str,
        project: &str,
        iid: u64,
    ) -> Result<MergeRequestDetail, Failure>;

    async fn list_commits(
        &self,
        base_url: &str,
        pat: &str,
        project: &str,
        iid: u64,
    ) -> Result<Vec<MergeRequestCommit>, Failure>;
}

pub trait ObjectFetch {
    async fn fetch(
        &self,
        repo_path: &Path,
        remote_url: &str,
        iid: u64,
        shas: &[String],
        specs: &[(String, String, String, String)],
    ) -> Result<Vec<CommitInfo>, Failure>;
}

pub async fn resolve_mr_entry<F, O>(
    repo_path: &Path,
    iid: u64,
    base_url: &str,
    pat: &str,
    forge: &F,
    objects: &O,
) -> Result<Ready, Failure>
where
    F: Forge,
    O: ObjectFetch,
{
    let project = forge.resolve(repo_path, base_url, pat).await?;
    let detail = forge
        .fetch_merge_request(base_url, pat, &project.path_with_namespace, iid)
        .await?;
    let commits = forge
        .list_commits(base_url, pat, &project.path_with_namespace, iid)
        .await?;
    let mut shas: Vec<String> = commits.iter().map(|c| c.id.clone()).collect();
    shas.push(detail.diff_refs.base_sha.clone());
    shas.push(detail.diff_refs.head_sha.clone());
    if let Some(start) = &detail.diff_refs.start_sha {
        shas.push(start.clone());
    }
    shas.sort();
    shas.dedup();
    let specs: Vec<(String, String, String, String)> = commits
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                c.title.clone(),
                c.author_name.clone(),
                c.authored_date.clone(),
            )
        })
        .collect();
    let commit_infos = objects
        .fetch(repo_path, &project.remote_url, iid, &shas, &specs)
        .await?;
    Ok(Ready {
        detail,
        project: project.path_with_namespace,
        commit_infos,
    })
}

pub struct GitLabForge {
    pub http: Arc<dyn gpui::http_client::HttpClient>,
}

impl Forge for GitLabForge {
    async fn resolve(
        &self,
        repo_path: &Path,
        base_url: &str,
        pat: &str,
    ) -> Result<GitLabProjectIdentity, Failure> {
        match gitlab::resolve_project(self.http.clone(), base_url, pat, repo_path).await {
            gitlab::ResolveProjectResult::Ok(identity) => Ok(identity),
            gitlab::ResolveProjectResult::Err(e) => Err(Failure {
                message: gitlab::format_resolve_project_error(&e),
                settings_target: e.settings_fix(),
            }),
        }
    }

    async fn fetch_merge_request(
        &self,
        base_url: &str,
        pat: &str,
        project: &str,
        iid: u64,
    ) -> Result<MergeRequestDetail, Failure> {
        match gitlab::fetch_merge_request(self.http.clone(), base_url, pat, project, iid).await {
            gitlab::FetchMergeRequestResult::Ok(detail) => Ok(detail),
            gitlab::FetchMergeRequestResult::Err(e) => Err(Failure {
                message: gitlab::format_fetch_merge_request_error(&e),
                settings_target: e.settings_fix(),
            }),
        }
    }

    async fn list_commits(
        &self,
        base_url: &str,
        pat: &str,
        project: &str,
        iid: u64,
    ) -> Result<Vec<MergeRequestCommit>, Failure> {
        match gitlab::list_merge_request_commits(self.http.clone(), base_url, pat, project, iid)
            .await
        {
            gitlab::ListMergeRequestCommitsResult::Ok(commits) => Ok(commits),
            gitlab::ListMergeRequestCommitsResult::Err(e) => Err(Failure {
                message: gitlab::format_list_merge_request_commits_error(&e),
                settings_target: e.settings_fix(),
            }),
        }
    }
}

pub struct SystemGitObjects;

impl ObjectFetch for SystemGitObjects {
    async fn fetch(
        &self,
        repo_path: &Path,
        remote_url: &str,
        iid: u64,
        shas: &[String],
        specs: &[(String, String, String, String)],
    ) -> Result<Vec<CommitInfo>, Failure> {
        git::fetch_oids(repo_path, remote_url, iid, shas).map_err(|e| Failure {
            message: e.0,
            settings_target: None,
        })?;
        git::commit_infos_from_mr_specs(repo_path, specs).map_err(|e| Failure {
            message: e.0,
            settings_target: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::{MergeRequestCheckState, MrDiffRefs};
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    fn detail(iid: u64, base: &str, head: &str, start: Option<&str>) -> MergeRequestDetail {
        MergeRequestDetail {
            iid,
            title: "MR".into(),
            description: None,
            author_username: "ada".into(),
            state: "opened".into(),
            source_branch: "feat".into(),
            target_branch: "main".into(),
            merge_status: None,
            diff_refs: MrDiffRefs {
                base_sha: base.into(),
                head_sha: head.into(),
                start_sha: start.map(str::to_string),
            },
            check_state: MergeRequestCheckState {
                pipeline_status: None,
                approved: None,
                approvals_label: None,
            },
        }
    }

    fn commit_info(summary: &str) -> CommitInfo {
        CommitInfo {
            oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            summary: summary.into(),
            body: String::new(),
            author: "ada".into(),
            time_label: "2020-01-01".into(),
        }
    }

    fn failure(message: &str) -> Failure {
        Failure {
            message: message.into(),
            settings_target: Some(gitlab::SettingsTarget::GitLabToken),
        }
    }

    struct FakeForge {
        resolve: Result<GitLabProjectIdentity, Failure>,
        fetch: Result<MergeRequestDetail, Failure>,
        commits: Result<Vec<MergeRequestCommit>, Failure>,
        fetch_calls: Cell<usize>,
        list_calls: Cell<usize>,
    }

    impl Forge for FakeForge {
        async fn resolve(
            &self,
            _repo_path: &Path,
            _base_url: &str,
            _pat: &str,
        ) -> Result<GitLabProjectIdentity, Failure> {
            self.resolve.clone()
        }

        async fn fetch_merge_request(
            &self,
            _base_url: &str,
            _pat: &str,
            _project: &str,
            _iid: u64,
        ) -> Result<MergeRequestDetail, Failure> {
            self.fetch_calls.set(self.fetch_calls.get() + 1);
            self.fetch.clone()
        }

        async fn list_commits(
            &self,
            _base_url: &str,
            _pat: &str,
            _project: &str,
            _iid: u64,
        ) -> Result<Vec<MergeRequestCommit>, Failure> {
            self.list_calls.set(self.list_calls.get() + 1);
            self.commits.clone()
        }
    }

    struct FakeObjects {
        result: Result<Vec<CommitInfo>, Failure>,
        calls: Cell<usize>,
        shas: RefCell<Vec<Vec<String>>>,
    }

    impl ObjectFetch for FakeObjects {
        async fn fetch(
            &self,
            _repo_path: &Path,
            _remote_url: &str,
            _iid: u64,
            shas: &[String],
            _specs: &[(String, String, String, String)],
        ) -> Result<Vec<CommitInfo>, Failure> {
            self.calls.set(self.calls.get() + 1);
            self.shas.borrow_mut().push(shas.to_vec());
            self.result.clone()
        }
    }

    #[test]
    fn resolve_mr_entry_returns_detail_project_and_commit_infos() {
        let detail = detail(7, "base", "head", None);
        let infos = vec![commit_info("fix thing")];
        let forge = FakeForge {
            resolve: Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
                remote_url: "https://gitlab.example/acme/widget.git".into(),
            }),
            fetch: Ok(detail.clone()),
            commits: Ok(vec![MergeRequestCommit {
                id: "abc".into(),
                title: "fix thing".into(),
                author_name: "Ada".into(),
                authored_date: "2020-01-02T03:04:05Z".into(),
            }]),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Ok(infos.clone()),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        let result = futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            7,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ));
        assert_eq!(
            result,
            Ok(Ready {
                detail,
                project: "acme/widget".into(),
                commit_infos: infos,
            })
        );
    }

    #[test]
    fn resolve_mr_entry_returns_resolve_failure_without_later_calls() {
        let err = failure("no remote");
        let forge = FakeForge {
            resolve: Err(err.clone()),
            fetch: Ok(detail(1, "base", "head", None)),
            commits: Ok(vec![]),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Ok(vec![]),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        let result = futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            1,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ));
        assert_eq!(result, Err(err));
        assert_eq!(forge.fetch_calls.get(), 0);
        assert_eq!(forge.list_calls.get(), 0);
        assert_eq!(objects.calls.get(), 0);
    }

    #[test]
    fn resolve_mr_entry_returns_fetch_failure_without_commits_or_objects() {
        let err = failure("mr missing");
        let forge = FakeForge {
            resolve: Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
                remote_url: "https://gitlab.example/acme/widget.git".into(),
            }),
            fetch: Err(err.clone()),
            commits: Ok(vec![]),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Ok(vec![]),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        let result = futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            3,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ));
        assert_eq!(result, Err(err));
        assert_eq!(forge.list_calls.get(), 0);
        assert_eq!(objects.calls.get(), 0);
    }

    #[test]
    fn resolve_mr_entry_returns_list_commits_failure_without_object_fetch() {
        let err = failure("commits missing");
        let forge = FakeForge {
            resolve: Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
                remote_url: "https://gitlab.example/acme/widget.git".into(),
            }),
            fetch: Ok(detail(4, "base", "head", None)),
            commits: Err(err.clone()),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Ok(vec![]),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        let result = futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            4,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ));
        assert_eq!(result, Err(err));
        assert_eq!(objects.calls.get(), 0);
    }

    #[test]
    fn resolve_mr_entry_returns_object_fetch_failure() {
        let err = failure("fetch failed");
        let forge = FakeForge {
            resolve: Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
                remote_url: "https://gitlab.example/acme/widget.git".into(),
            }),
            fetch: Ok(detail(5, "base", "head", Some("start"))),
            commits: Ok(vec![MergeRequestCommit {
                id: "abc".into(),
                title: "fix thing".into(),
                author_name: "Ada".into(),
                authored_date: "2020-01-02T03:04:05Z".into(),
            }]),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Err(err.clone()),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        let result = futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            5,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ));
        assert_eq!(result, Err(err));
    }

    fn object_fetch_shas(
        commit_ids: &[&str],
        base: &str,
        head: &str,
        start: Option<&str>,
    ) -> Vec<Vec<String>> {
        let forge = FakeForge {
            resolve: Ok(GitLabProjectIdentity {
                path_with_namespace: "acme/widget".into(),
                remote_url: "https://gitlab.example/acme/widget.git".into(),
            }),
            fetch: Ok(detail(9, base, head, start)),
            commits: Ok(commit_ids
                .iter()
                .map(|id| MergeRequestCommit {
                    id: (*id).into(),
                    title: "t".into(),
                    author_name: "Ada".into(),
                    authored_date: "2020-01-02T03:04:05Z".into(),
                })
                .collect()),
            fetch_calls: Cell::new(0),
            list_calls: Cell::new(0),
        };
        let objects = FakeObjects {
            result: Ok(vec![]),
            calls: Cell::new(0),
            shas: RefCell::new(Vec::new()),
        };
        futures::executor::block_on(resolve_mr_entry(
            Path::new("/repo"),
            9,
            "https://gitlab.example",
            "tok",
            &forge,
            &objects,
        ))
        .unwrap();
        objects.shas.borrow().clone()
    }

    #[test]
    fn resolve_mr_entry_passes_sorted_deduped_shas_to_one_object_fetch() {
        assert_eq!(
            object_fetch_shas(&["c1", "c0"], "base", "c1", Some("start")),
            vec![vec![
                "base".to_string(),
                "c0".to_string(),
                "c1".to_string(),
                "start".to_string(),
            ]]
        );
        assert_eq!(
            object_fetch_shas(&["m", "a"], "b", "z", None),
            vec![vec![
                "a".to_string(),
                "b".to_string(),
                "m".to_string(),
                "z".to_string(),
            ]]
        );
    }
}
