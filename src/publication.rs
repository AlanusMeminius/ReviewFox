//! The originating forge target, separate from Comparison identity and credentials.
use crate::domain::{Comparison, Oid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
