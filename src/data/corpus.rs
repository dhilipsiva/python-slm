use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"RLCORP02";
const FOOTER_BYTES: usize = 4 + 8 + 8 + 8;
const MAX_DOCUMENT_ID_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_DOCUMENT_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub text: String,
}

pub struct CorpusWriter {
    file: BufWriter<File>,
    final_path: PathBuf,
    partial_path: PathBuf,
    records: u64,
    bytes: u64,
}

impl CorpusWriter {
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        ensure!(
            !path.exists(),
            "refusing to overwrite existing corpus {}",
            path.display()
        );
        let partial_path = partial_path(path);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial_path)
            .with_context(|| {
                format!(
                    "creating partial corpus {} (remove it explicitly after inspecting a failed run)",
                    partial_path.display()
                )
            })?;
        let mut file = BufWriter::with_capacity(8 * 1024 * 1024, file);
        file.write_all(MAGIC)?;
        Ok(Self {
            file,
            final_path: path.to_owned(),
            partial_path,
            records: 0,
            bytes: 0,
        })
    }

    pub fn write(&mut self, document: &Document) -> Result<()> {
        let id = document.id.as_bytes();
        let text = document.text.as_bytes();
        ensure!(!id.is_empty(), "document id must not be empty");
        ensure!(
            id.len() <= MAX_DOCUMENT_ID_BYTES,
            "document id is too large"
        );
        ensure!(
            text.len() <= MAX_DOCUMENT_TEXT_BYTES,
            "document text exceeds the 64 MiB safety limit"
        );
        self.file.write_all(&(id.len() as u32).to_le_bytes())?;
        self.file.write_all(&(text.len() as u64).to_le_bytes())?;
        self.file.write_all(id)?;
        self.file.write_all(text)?;
        self.records += 1;
        self.bytes += text.len() as u64;
        Ok(())
    }

    pub fn finish(self) -> Result<(u64, u64)> {
        let Self {
            mut file,
            final_path,
            partial_path,
            records,
            bytes,
        } = self;
        // An empty id is forbidden for records, so the zero lengths form an
        // unambiguous completion frame followed by redundant integrity counts.
        file.write_all(&0_u32.to_le_bytes())?;
        file.write_all(&0_u64.to_le_bytes())?;
        file.write_all(&records.to_le_bytes())?;
        file.write_all(&bytes.to_le_bytes())?;
        file.flush()?;
        file.get_ref().sync_all()?;
        drop(file);
        std::fs::rename(&partial_path, &final_path).with_context(|| {
            format!(
                "atomically finalizing {} as {}",
                partial_path.display(),
                final_path.display()
            )
        })?;
        Ok((records, bytes))
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(".part");
    PathBuf::from(value)
}

pub struct CorpusReader {
    file: BufReader<File>,
    finished: bool,
    records: u64,
    bytes: u64,
}

impl CorpusReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = BufReader::with_capacity(
            8 * 1024 * 1024,
            File::open(path).with_context(|| format!("opening {}", path.display()))?,
        );
        let mut magic = [0_u8; 8];
        file.read_exact(&mut magic)
            .with_context(|| format!("reading corpus header from {}", path.display()))?;
        ensure!(
            &magic == MAGIC,
            "invalid corpus magic in {}",
            path.display()
        );
        Ok(Self {
            file,
            finished: false,
            records: 0,
            bytes: 0,
        })
    }
}

impl Iterator for CorpusReader {
    type Item = Result<Document>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let mut id_len = [0_u8; 4];
        match self.file.read(&mut id_len[..1]) {
            Ok(0) => {
                self.finished = true;
                return Some(Err(anyhow::anyhow!(
                    "corpus ended before its completion footer"
                )));
            }
            Ok(1) => {}
            Ok(_) => unreachable!("a one-byte read cannot return more than one byte"),
            Err(error) => return Some(Err(error.into())),
        }
        if let Err(error) = self.file.read_exact(&mut id_len[1..]) {
            return Some(Err(error).context("truncated corpus record header"));
        }
        let mut text_len = [0_u8; 8];
        if let Err(error) = self.file.read_exact(&mut text_len) {
            return Some(Err(error.into()));
        }
        let id_len = u32::from_le_bytes(id_len) as usize;
        let text_len_u64 = u64::from_le_bytes(text_len);
        if id_len == 0 && text_len_u64 == 0 {
            let mut footer = [0_u8; FOOTER_BYTES - 12];
            if let Err(error) = self.file.read_exact(&mut footer) {
                self.finished = true;
                return Some(Err(error).context("truncated corpus completion footer"));
            }
            let expected_records = u64::from_le_bytes(footer[..8].try_into().unwrap());
            let expected_bytes = u64::from_le_bytes(footer[8..].try_into().unwrap());
            let mut trailing = [0_u8; 1];
            let trailing_result = self.file.read(&mut trailing);
            self.finished = true;
            if expected_records != self.records || expected_bytes != self.bytes {
                return Some(Err(anyhow::anyhow!(
                    "corpus footer mismatch: expected {expected_records} records/{expected_bytes} bytes, decoded {}/{}",
                    self.records,
                    self.bytes
                )));
            }
            return match trailing_result {
                Ok(0) => None,
                Ok(_) => Some(Err(anyhow::anyhow!(
                    "trailing bytes after corpus completion footer"
                ))),
                Err(error) => Some(Err(error.into())),
            };
        }
        if id_len == 0 {
            self.finished = true;
            return Some(Err(anyhow::anyhow!("invalid zero-length corpus id")));
        }
        if text_len_u64 > usize::MAX as u64 {
            return Some(Err(anyhow::anyhow!(
                "corpus record does not fit address space"
            )));
        }
        let text_len = text_len_u64 as usize;
        if id_len > MAX_DOCUMENT_ID_BYTES || text_len > MAX_DOCUMENT_TEXT_BYTES {
            self.finished = true;
            return Some(Err(anyhow::anyhow!(
                "corpus record exceeds configured allocation safety limits"
            )));
        }
        let mut id = vec![0_u8; id_len];
        let mut text = vec![0_u8; text_len];
        if let Err(error) = self.file.read_exact(&mut id) {
            return Some(Err(error.into()));
        }
        if let Err(error) = self.file.read_exact(&mut text) {
            return Some(Err(error.into()));
        }
        let id = match String::from_utf8(id) {
            Ok(id) => id,
            Err(error) => return Some(Err(error.into())),
        };
        let text = match String::from_utf8(text) {
            Ok(text) => text,
            Err(error) => return Some(Err(error.into())),
        };
        if id.is_empty() {
            self.finished = true;
            return Some(Err(anyhow::anyhow!("empty document id in corpus")));
        }
        self.records += 1;
        self.bytes += text_len as u64;
        Some(Ok(Document { id, text }))
    }
}

pub fn require_nonempty_corpus(path: &Path) -> Result<()> {
    let mut reader = CorpusReader::open(path)?;
    if reader.next().transpose()?.is_none() {
        bail!("corpus {} has no records", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_round_trip_and_overwrite_refusal() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.corpus");
        let documents = [
            Document {
                id: "one".into(),
                text: "print(1)".into(),
            },
            Document {
                id: "two".into(),
                text: "def f():\n    return 2\n".into(),
            },
        ];
        let mut writer = CorpusWriter::create(&path).unwrap();
        for document in &documents {
            writer.write(document).unwrap();
        }
        assert!(!path.exists());
        assert!(partial_path(&path).exists());
        assert_eq!(writer.finish().unwrap(), (2, 30));
        assert!(!partial_path(&path).exists());
        assert!(CorpusWriter::create(&path).is_err());

        let actual: Vec<_> = CorpusReader::open(&path)
            .unwrap()
            .map(|document| document.unwrap())
            .collect();
        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0].id, "one");
        assert_eq!(actual[1].text, documents[1].text);
    }

    #[test]
    fn corpus_without_completion_footer_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("truncated.corpus");
        std::fs::write(&path, MAGIC).unwrap();
        let mut reader = CorpusReader::open(&path).unwrap();
        assert!(reader.next().unwrap().is_err());
        assert!(reader.next().is_none());
    }
}
