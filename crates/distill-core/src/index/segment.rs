//! One segment of the word index: an immutable file holding, for a set of sessions, every word
//! and the sessions it is in. A segment is written once, in word order, and never changed; the
//! manifest says which of its sessions are still current.
//!
//! Layout, integers little-endian:
//!
//! ```text
//! magic (8 bytes)
//! u32 session count, then each session's key: u32 length, bytes      (ordinal = position)
//! postings: one run per word, in word order
//! words: each word's bytes, in word order
//! table: per word, u64 word offset (into words), u32 word length,
//!        u64 postings offset (into the file), u32 postings length
//! footer: u64 word count, u64 words start, u64 table start, magic
//! ```
//!
//! A run of postings is, per session in ordinal order: varint ordinal, varint count of the word
//! in that session, varint number of events, then each event's sequence number as a varint delta.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

const MAGIC: &[u8; 8] = b"DSTLWRD1";
const ENTRY: u64 = 24;
const FOOTER: u64 = 32;

/// A word's occurrences in one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    /// The session, by its position in the segment.
    pub ord: u32,
    /// How many times the word occurs in the session.
    pub count: u32,
    /// The first events it occurs in, by sequence number, ascending.
    pub events: Vec<u64>,
}

/// `postings`, in ordinal order, as one run.
pub fn encode(postings: &[Posting]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in postings {
        varint(&mut out, u64::from(p.ord));
        varint(&mut out, u64::from(p.count));
        varint(&mut out, p.events.len() as u64);
        let mut last = 0;
        for &e in &p.events {
            varint(&mut out, e - last);
            last = e;
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Posting>> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < bytes.len() {
        let ord = u32::try_from(unvarint(bytes, &mut at)?)?;
        let count = u32::try_from(unvarint(bytes, &mut at)?)?;
        let n = unvarint(bytes, &mut at)?;
        let mut events = Vec::with_capacity(n.min(64) as usize);
        let mut last = 0u64;
        for _ in 0..n {
            last = last
                .checked_add(unvarint(bytes, &mut at)?)
                .context("an event number overflows")?;
            events.push(last);
        }
        out.push(Posting { ord, count, events });
    }
    Ok(out)
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn unvarint(bytes: &[u8], at: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *bytes.get(*at).context("postings end inside a number")?;
        *at += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    bail!("a number in the postings is too long")
}

/// Writes a segment, one word at a time in word order. Nothing is in place until [`finish`].
///
/// [`finish`]: Writer::finish
pub struct Writer {
    path: PathBuf,
    tmp: PathBuf,
    out: BufWriter<File>,
    pos: u64,
    words: BufWriter<File>,
    words_len: u64,
    table: BufWriter<File>,
    count: u64,
    last: Option<String>,
}

impl Writer {
    /// Start the segment at `path` for `sessions`, whose positions are their ordinals. Its
    /// working files sit beside it, never outside the home.
    pub fn create(path: &Path, sessions: &[String]) -> Result<Self> {
        let dir = path.parent().context("a segment path has no folder")?;
        let tmp = path.with_extension("tmp");
        let mut out = BufWriter::new(
            crate::util::private_file()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?,
        );
        out.write_all(MAGIC)?;
        out.write_all(&u32::try_from(sessions.len())?.to_le_bytes())?;
        let mut pos = 12;
        for s in sessions {
            out.write_all(&u32::try_from(s.len())?.to_le_bytes())?;
            out.write_all(s.as_bytes())?;
            pos += 4 + s.len() as u64;
        }
        Ok(Self {
            path: path.to_path_buf(),
            tmp,
            out,
            pos,
            words: BufWriter::new(tempfile::tempfile_in(dir)?),
            words_len: 0,
            table: BufWriter::new(tempfile::tempfile_in(dir)?),
            count: 0,
            last: None,
        })
    }

    /// Add `word` with its run of postings. Words must come in ascending order.
    pub fn add(&mut self, word: &str, run: &[u8]) -> Result<()> {
        ensure!(
            self.last.as_deref().is_none_or(|l| l < word),
            "words must be added in order"
        );
        self.out.write_all(run)?;
        self.words.write_all(word.as_bytes())?;
        self.table.write_all(&self.words_len.to_le_bytes())?;
        self.table
            .write_all(&u32::try_from(word.len())?.to_le_bytes())?;
        self.table.write_all(&self.pos.to_le_bytes())?;
        self.table
            .write_all(&u32::try_from(run.len())?.to_le_bytes())?;
        self.pos += run.len() as u64;
        self.words_len += word.len() as u64;
        self.count += 1;
        self.last = Some(word.to_string());
        Ok(())
    }

    /// Write the words, the table and the footer, and put the segment in place. Returns its size.
    pub fn finish(self) -> Result<u64> {
        let Self {
            path,
            tmp,
            mut out,
            pos,
            words,
            table,
            count,
            ..
        } = self;
        let words_start = pos;
        let mut words = words.into_inner()?;
        words.seek(SeekFrom::Start(0))?;
        let words_len = std::io::copy(&mut words, &mut out)?;
        let table_start = words_start + words_len;
        let mut table = table.into_inner()?;
        table.seek(SeekFrom::Start(0))?;
        let table_len = std::io::copy(&mut table, &mut out)?;
        out.write_all(&count.to_le_bytes())?;
        out.write_all(&words_start.to_le_bytes())?;
        out.write_all(&table_start.to_le_bytes())?;
        out.write_all(MAGIC)?;
        let file = out.into_inner()?;
        file.sync_all()?;
        fs::rename(&tmp, &path).with_context(|| format!("put {} in place", path.display()))?;
        Ok(table_start + table_len + FOOTER)
    }
}

/// A segment open for reading.
pub struct Segment {
    path: PathBuf,
    file: File,
    /// Each session's key, by ordinal.
    pub sessions: Vec<String>,
    postings_start: u64,
    count: u64,
    words_start: u64,
    table_start: u64,
}

impl Segment {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let len = file.metadata()?.len();
        ensure!(len >= 12 + FOOTER, "{} is not a segment", path.display());
        let mut r = BufReader::new(&file);
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        ensure!(&magic == MAGIC, "{} is not a segment", path.display());
        let n = u32_at(&mut r)?;
        let mut sessions = Vec::with_capacity(n as usize);
        let mut pos = 12u64;
        for _ in 0..n {
            let l = u32_at(&mut r)? as usize;
            let mut key = vec![0u8; l];
            r.read_exact(&mut key)?;
            sessions.push(String::from_utf8(key)?);
            pos += 4 + l as u64;
        }
        let mut footer = [0u8; FOOTER as usize];
        (&file).seek(SeekFrom::Start(len - FOOTER))?;
        (&file).read_exact(&mut footer)?;
        ensure!(&footer[24..] == MAGIC, "{} is cut short", path.display());
        let at = |i: usize| u64::from_le_bytes(footer[i..i + 8].try_into().unwrap());
        let (count, words_start, table_start) = (at(0), at(8), at(16));
        ensure!(
            pos <= words_start
                && words_start <= table_start
                && table_start + count * ENTRY + FOOTER == len,
            "{} does not add up",
            path.display()
        );
        Ok(Self {
            path: path.to_path_buf(),
            file,
            sessions,
            postings_start: pos,
            count,
            words_start,
            table_start,
        })
    }

    /// The run of postings for `word`, if the segment has it.
    pub fn get(&self, word: &str) -> Result<Option<Vec<u8>>> {
        let (mut lo, mut hi) = (0u64, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let e = self.entry(mid)?;
            let w = self.bytes(self.words_start + e.word, e.word_len)?;
            match w.as_slice().cmp(word.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(Some(self.bytes(e.run, e.run_len)?)),
            }
        }
        Ok(None)
    }

    /// Every word with its run of postings, in word order.
    pub fn words(&self) -> Result<Words> {
        let open = |at: u64| -> Result<BufReader<File>> {
            let mut f = File::open(&self.path)?;
            f.seek(SeekFrom::Start(at))?;
            Ok(BufReader::new(f))
        };
        Ok(Words {
            table: open(self.table_start)?,
            words: open(self.words_start)?,
            runs: open(self.postings_start)?,
            left: self.count,
        })
    }

    fn entry(&self, i: u64) -> Result<Entry> {
        let b = self.bytes(self.table_start + i * ENTRY, ENTRY as u32)?;
        Ok(Entry::from(&b))
    }

    fn bytes(&self, at: u64, len: u32) -> Result<Vec<u8>> {
        let mut out = vec![0u8; len as usize];
        (&self.file).seek(SeekFrom::Start(at))?;
        (&self.file).read_exact(&mut out)?;
        Ok(out)
    }
}

struct Entry {
    word: u64,
    word_len: u32,
    run: u64,
    run_len: u32,
}

impl Entry {
    fn from(b: &[u8]) -> Self {
        Self {
            word: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            word_len: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            run: u64::from_le_bytes(b[12..20].try_into().unwrap()),
            run_len: u32::from_le_bytes(b[20..24].try_into().unwrap()),
        }
    }
}

fn u32_at(r: &mut impl Read) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// A segment's words in order, read front to back.
pub struct Words {
    table: BufReader<File>,
    words: BufReader<File>,
    runs: BufReader<File>,
    left: u64,
}

impl Iterator for Words {
    type Item = Result<(String, Vec<u8>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let mut read = || -> Result<(String, Vec<u8>)> {
            let mut b = [0u8; ENTRY as usize];
            self.table.read_exact(&mut b)?;
            let e = Entry::from(&b);
            let mut word = vec![0u8; e.word_len as usize];
            self.words.read_exact(&mut word)?;
            let mut run = vec![0u8; e.run_len as usize];
            self.runs.read_exact(&mut run)?;
            Ok((String::from_utf8(word)?, run))
        };
        Some(read())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_reads_back_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seg-1.bin");
        let sessions = vec!["claude-code/a".to_string(), "codex/b".to_string()];
        let p = |ord, count, events: &[u64]| Posting {
            ord,
            count,
            events: events.to_vec(),
        };
        let words = [
            ("alpha", vec![p(0, 3, &[1, 4, 300])]),
            ("beta", vec![p(0, 1, &[2]), p(1, 2, &[0, 7])]),
            ("zed", vec![p(1, 1, &[1_000_000])]),
        ];
        let mut w = Writer::create(&path, &sessions).unwrap();
        for (word, postings) in &words {
            w.add(word, &encode(postings)).unwrap();
        }
        assert!(w.add("aardvark", &[]).is_err(), "out of order");
        let size = w.finish().unwrap();
        assert_eq!(size, fs::metadata(&path).unwrap().len());
        // Only the segment is left: its working files are gone.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);

        let s = Segment::open(&path).unwrap();
        assert_eq!(s.sessions, sessions);
        for (word, postings) in &words {
            let run = s.get(word).unwrap().unwrap();
            assert_eq!(&decode(&run).unwrap(), postings, "{word}");
        }
        for missing in ["", "a", "alphas", "c", "zzz"] {
            assert!(s.get(missing).unwrap().is_none(), "{missing}");
        }
        let all: Vec<(String, Vec<Posting>)> = s
            .words()
            .unwrap()
            .map(|r| {
                let (w, run) = r.unwrap();
                (w, decode(&run).unwrap())
            })
            .collect();
        let want: Vec<(String, Vec<Posting>)> = words
            .iter()
            .map(|(w, p)| (w.to_string(), p.clone()))
            .collect();
        assert_eq!(all, want);
    }

    #[test]
    fn a_cut_or_foreign_file_is_not_a_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seg-1.bin");
        let mut w = Writer::create(&path, &["a".to_string()]).unwrap();
        w.add(
            "x",
            &encode(&[Posting {
                ord: 0,
                count: 1,
                events: vec![0],
            }]),
        )
        .unwrap();
        w.finish().unwrap();
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        assert!(Segment::open(&path).is_err());
        fs::write(&path, b"not a segment at all, just some text here").unwrap();
        assert!(Segment::open(&path).is_err());
    }
}
