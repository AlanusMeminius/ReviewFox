//! Versioned, atomic local Review snapshots. Errors never replace unreadable work.
use crate::{
    domain::{Comparison, Review},
    export::ReviewContexts,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ReviewStore {
    directory: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct SavedReview {
    pub version: u32,
    pub review: Review,
    pub contexts: ReviewContexts,
}

impl ReviewStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn application() -> Result<Self> {
        Ok(Self::new(
            dirs::data_dir()
                .context("Review storage directory unavailable")?
                .join("ReviewFox")
                .join("reviews"),
        ))
    }
    fn path(&self, comparison: &Comparison) -> Result<PathBuf> {
        let identity = serde_json::to_vec(comparison)?;
        let hash = git2::Oid::hash_object(git2::ObjectType::Blob, &identity)?;
        Ok(self.directory.join(format!("{hash}.json")))
    }
    pub fn load(&self, comparison: &Comparison) -> Result<Option<SavedReview>> {
        let path = self.path(comparison)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("Cannot read {}", path.display())),
        };
        let saved: SavedReview = serde_json::from_slice(&bytes)
            .with_context(|| format!("Unreadable Review preserved at {}", path.display()))?;
        if saved.version != 1 || &saved.review.comparison != comparison {
            bail!(
                "Unsupported or mismatched Review preserved at {}",
                path.display()
            );
        }
        if let Err(reason) = saved.review.validate() {
            bail!("{reason}; Review preserved at {}", path.display());
        }
        if saved
            .contexts
            .keys()
            .any(|id| !saved.review.comments.iter().any(|c| c.id == *id))
        {
            bail!(
                "Mismatched creation context; Review preserved at {}",
                path.display()
            );
        }
        Ok(Some(saved))
    }
    pub fn save(&self, review: &Review, contexts: &ReviewContexts) -> Result<()> {
        review.validate().map_err(anyhow::Error::msg)?;
        // Validate an existing snapshot before replacing it, including future versions.
        self.load(&review.comparison)?;
        std::fs::create_dir_all(&self.directory)?;
        let path = self.path(&review.comparison)?;
        let saved = SavedReview {
            version: 1,
            review: review.clone(),
            contexts: contexts.clone(),
        };
        let bytes = serde_json::to_vec(&saved)?;
        atomic_save(&path, &bytes)
    }
}

/// Replace a snapshot only after its complete content is flushed.
pub fn atomic_save(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("Cannot save local work to {}", path.display()))?;
    Ok(())
}
