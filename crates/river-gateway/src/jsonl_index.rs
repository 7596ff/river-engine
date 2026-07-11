//! Incremental JSONL reader shared by record, move, and channel indexes.
//! Engine-owned appends advance in memory after fsync. Unannounced
//! growth, truncation, replacement, or same-size modification rebuilds
//! from a stable snapshot.

use std::fs::{File, Metadata, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context as _;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    identity: FileIdentity,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity;

impl FileStamp {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            identity: file_identity(metadata),
        }
    }
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt as _;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(_metadata: &Metadata) -> FileIdentity {
    FileIdentity
}

/// Long-lived append handles otherwise keep writing an unlinked inode
/// after a hand replacement (`rename` over the path). Reopen whenever
/// the visible path no longer names the handle's file.
pub fn ensure_append_target(file: &mut File, path: &Path) -> anyhow::Result<()> {
    let reopen = match std::fs::metadata(path) {
        Ok(path_metadata) => file_identity(&file.metadata()?) != file_identity(&path_metadata),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if reopen {
        *file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .with_context(|| format!("reopening {} after replacement", path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Unchanged,
    Rebuilt,
}

pub struct JsonlIndex<T> {
    path: PathBuf,
    description: &'static str,
    stamp: Option<FileStamp>,
    items: Vec<T>,
    committed_items: usize,
    committed_offset: u64,
    committed_lines: usize,
    tail: Vec<u8>,
}

impl<T> JsonlIndex<T>
where
    T: Clone + DeserializeOwned,
{
    pub fn new(path: PathBuf, description: &'static str) -> Self {
        Self {
            path,
            description,
            stamp: None,
            items: Vec::new(),
            committed_items: 0,
            committed_offset: 0,
            committed_lines: 0,
            tail: Vec::new(),
        }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn refresh(&mut self) -> anyhow::Result<Refresh> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.stamp.is_none() && self.items.is_empty() {
                    return Ok(Refresh::Unchanged);
                }
                self.clear();
                return Ok(Refresh::Rebuilt);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", self.path.display()));
            }
        };
        let current = FileStamp::from_metadata(&metadata);
        let Some(previous) = &self.stamp else {
            self.rebuild()?;
            return Ok(Refresh::Rebuilt);
        };

        if current == *previous {
            return Ok(Refresh::Unchanged);
        }
        // Engine writers advance the cache synchronously through
        // `apply_known_append`. Any metadata change that reaches this
        // path is therefore external or ambiguous and rebuilds.
        self.rebuild()?;
        Ok(Refresh::Rebuilt)
    }

    /// Advance an initialized index after its single writer durably
    /// appends exactly one serialized line. The before/after metadata
    /// proves no unobserved edit occurred between the cached snapshot
    /// and this write. False means the caller must discard the cache.
    pub fn apply_known_append(
        &mut self,
        item: T,
        serialized: &[u8],
        before: &Metadata,
        after: &Metadata,
    ) -> anyhow::Result<bool> {
        let Some(previous) = &self.stamp else {
            return Ok(false);
        };
        let before = FileStamp::from_metadata(before);
        let after = FileStamp::from_metadata(after);
        let one_complete_line = serialized.ends_with(b"\n")
            && serialized.iter().filter(|&&byte| byte == b'\n').count() == 1;
        if before != *previous
            || before.identity != after.identity
            || after.len != before.len + serialized.len() as u64
            || !self.tail.is_empty()
            || !one_complete_line
        {
            return Ok(false);
        }

        self.items.push(item);
        self.committed_items = self.items.len();
        self.committed_offset = after.len;
        self.committed_lines += 1;
        self.stamp = Some(after);
        Ok(true)
    }

    fn rebuild(&mut self) -> anyhow::Result<()> {
        let (text, stamp) = stable_snapshot(&self.path)?;
        self.items.clear();
        self.committed_items = 0;
        self.committed_offset = 0;
        self.committed_lines = 0;
        self.tail.clear();
        self.parse_from(text.as_bytes(), 0, 0);
        self.stamp = Some(stamp);
        Ok(())
    }

    fn parse_from(&mut self, bytes: &[u8], base_offset: u64, base_line: usize) {
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %e,
                    "skipping invalid UTF-8 JSONL suffix"
                );
                self.tail = bytes.to_vec();
                return;
            }
        };
        self.tail.clear();
        let mut offset = base_offset;
        let mut line_no = base_line;
        for segment in text.split_inclusive('\n') {
            line_no += 1;
            let complete = segment.ends_with('\n');
            let raw = segment.strip_suffix('\n').unwrap_or(segment);
            if !raw.trim().is_empty() {
                match serde_json::from_str::<T>(raw) {
                    Ok(item) => self.items.push(item),
                    Err(e) => tracing::warn!(
                        path = %self.path.display(),
                        line = line_no,
                        error = %e,
                        description = self.description,
                        "skipping malformed JSONL line"
                    ),
                }
            }
            if complete {
                offset += segment.len() as u64;
                self.committed_offset = offset;
                self.committed_items = self.items.len();
                self.committed_lines = line_no;
            } else {
                self.tail = segment.as_bytes().to_vec();
            }
        }
    }

    fn clear(&mut self) {
        self.stamp = None;
        self.items.clear();
        self.committed_items = 0;
        self.committed_offset = 0;
        self.committed_lines = 0;
        self.tail.clear();
    }
}

fn stable_snapshot(path: &Path) -> anyhow::Result<(String, FileStamp)> {
    for _ in 0..3 {
        let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let before = FileStamp::from_metadata(&file.metadata()?);
        let mut text = String::new();
        file.read_to_string(&mut text)
            .with_context(|| format!("reading {}", path.display()))?;
        let after = FileStamp::from_metadata(&file.metadata()?);
        if before == after && text.len() as u64 == after.len {
            return Ok((text, after));
        }
    }
    anyhow::bail!("{} changed repeatedly while being indexed", path.display())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
    struct Item {
        value: String,
    }

    fn line(value: &str) -> String {
        format!("{{\"value\":\"{value}\"}}\n")
    }

    #[test]
    fn known_append_advances_without_a_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items.jsonl");
        std::fs::write(&path, line("one")).unwrap();
        let mut index = JsonlIndex::<Item>::new(path.clone(), "test item");
        assert_eq!(index.refresh().unwrap(), Refresh::Rebuilt);

        let serialized = line("two");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let before = file.metadata().unwrap();
        file.write_all(serialized.as_bytes()).unwrap();
        file.sync_data().unwrap();
        let after = file.metadata().unwrap();

        assert!(
            index
                .apply_known_append(
                    Item {
                        value: "two".into()
                    },
                    serialized.as_bytes(),
                    &before,
                    &after,
                )
                .unwrap()
        );
        assert_eq!(
            index.items(),
            [
                Item {
                    value: "one".into()
                },
                Item {
                    value: "two".into()
                }
            ]
        );
        assert_eq!(index.refresh().unwrap(), Refresh::Unchanged);
    }

    #[test]
    fn same_size_edit_and_replacement_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items.jsonl");
        std::fs::write(&path, line("one")).unwrap();
        let mut index = JsonlIndex::<Item>::new(path.clone(), "test item");
        index.refresh().unwrap();

        std::fs::write(&path, line("two")).unwrap();
        assert_eq!(index.refresh().unwrap(), Refresh::Rebuilt);
        assert_eq!(index.items()[0].value, "two");

        let replacement = dir.path().join("replacement.jsonl");
        std::fs::write(&replacement, line("new")).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(index.refresh().unwrap(), Refresh::Rebuilt);
        assert_eq!(index.items()[0].value, "new");
    }

    #[test]
    fn truncation_and_completed_tail_rebuild_safely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items.jsonl");
        std::fs::write(&path, "{\"value\":\"one\"").unwrap();
        let mut index = JsonlIndex::<Item>::new(path.clone(), "test item");
        index.refresh().unwrap();
        assert!(index.items().is_empty(), "torn tail is skipped");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"}\n").unwrap();
        file.sync_data().unwrap();
        assert_eq!(index.refresh().unwrap(), Refresh::Rebuilt);
        assert_eq!(index.items()[0].value, "one");

        std::fs::write(&path, "").unwrap();
        assert_eq!(index.refresh().unwrap(), Refresh::Rebuilt);
        assert!(index.items().is_empty());
    }
}
