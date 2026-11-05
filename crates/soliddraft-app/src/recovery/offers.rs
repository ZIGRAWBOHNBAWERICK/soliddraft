//! Decoding browser recovery offers without depending on the browser runtime.

use soliddraft_document::recovery::{self, RecoveryCopy};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// What storage held from earlier visits when autosave started.
pub struct Found {
    /// Readable copies, newest first.
    pub offers: Vec<RecoveryCopy>,
    /// Copies that could not be decoded, as sentences.
    pub problems: Vec<String>,
    pub(crate) bytes: HashMap<String, Vec<u8>>,
}

pub(crate) fn read(entries: impl IntoIterator<Item = (String, Vec<u8>)>) -> Found {
    let mut found = Found { offers: Vec::new(), problems: Vec::new(), bytes: HashMap::new() };
    for (key, copy) in entries {
        match recovery::decode(std::io::Cursor::new(&copy)) {
            Ok((record, _)) => {
                found.offers.push(RecoveryCopy { record, file: PathBuf::from(&key) });
                found.bytes.insert(key, copy);
            }
            Err(e) => {
                found.problems.push(format!("A recovery copy could not be read and was kept in browser storage ({e})"));
                found.bytes.insert(key, copy);
            }
        }
    }
    found.offers.sort_by_key(|c| std::cmp::Reverse(c.record.written_ms));
    let mut seen = HashSet::new();
    found.offers.retain(|copy| {
        if seen.insert((copy.record.id, copy.record.kind, copy.record.revision.clone())) {
            true
        } else {
            found.bytes.remove(copy.file.to_string_lossy().as_ref());
            false
        }
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_document::{
        ContentHash, DocKind, DocumentId,
        manifest::{Entry, Manifest, Section, write_package},
    };

    #[test]
    fn identical_revisions_keep_only_the_newest_readable_copy() {
        let id = DocumentId::new();
        let revision = ContentHash::of_bytes(b"work");
        let mut manifest = Manifest::new(id, DocKind::Part, revision.clone());
        manifest.sections = vec![Section::required("part.json")];
        let document = write_package(&manifest, &[Entry::new("part.json", b"work")]).unwrap();
        let mut record = recovery::Record::new(id, DocKind::Part, "Work", None, revision);
        record.written_ms = 1;
        let old = recovery::encode(&record, &document).unwrap();
        record.written_ms = 2;
        let new = recovery::encode(&record, &document).unwrap();
        let found = read([("old".into(), old), ("new".into(), new.clone())]);
        assert_eq!(found.offers.len(), 1);
        assert_eq!(found.offers[0].record, record);
        assert_eq!(found.bytes.get("new"), Some(&new));
        assert!(!found.bytes.contains_key("old"));
    }

    #[test]
    fn unsupported_recovery_versions_keep_the_only_unsaved_bytes() {
        let id = DocumentId::new();
        let revision = ContentHash::of_bytes(b"unsaved");
        let mut manifest = Manifest::new(id, DocKind::Part, revision.clone());
        manifest.sections = vec![Section::required("part.json")];
        let document = write_package(&manifest, &[Entry::new("part.json", b"unsaved")]).unwrap();
        let mut record = recovery::Record::new(id, DocKind::Part, "Future work", None, revision);
        record.format = "soliddraft-recovery/999".into();
        let copy = recovery::encode(&record, &document).unwrap();
        let found = read([("future".into(), copy.clone())]);
        assert_eq!(found.bytes.get("future"), Some(&copy));
        assert_eq!(found.problems.len(), 1);
    }
}
