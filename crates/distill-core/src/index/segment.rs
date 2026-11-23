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
//! words: in blocks of BLOCK words, in word order; per word, varint bytes it shares with the
//!        word before it in its block (0 for a block's first), varint length of the rest, the
//!        rest, varint length of its run of postings
//! blocks: per block, u64 offset of its words, u64 offset of its first word's run
//! footer: u64 word count, u64 words start, u64 blocks start, magic
//! ```
//!
//! A run of postings is, per session in ordinal order: varint ordinal (the first) or how far it
//! is past the one before (the rest), varint count of the word in that session, varint number of
//! events, then each event's sequence number as a varint delta.
//!
//! A word costs its bytes past what it shares with the word before it and three varints, plus a
//! block's entry per `BLOCK` words: no fixed-size entry per word (issue #154).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

const MAGIC: &[u8; 8] = b"DSTLWRD2";
/// Words per block. A lookup finds a word's block by binary search and reads the block through.
const BLOCK: u64 = if cfg!(test) { 4 } else { 32 };
/// What a block's entry takes.
const ENTRY: u64 = 16;
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

/// `postings`, in ascending ordinal order, as one run.
pub fn encode(postings: &[Posting]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut last_ord = 0;
    for p in postings {
        debug_assert!(p.ord >= last_ord, "postings in ordinal order");
        varint(&mut out, u64::from(p.ord - last_ord));
        last_ord = p.ord;
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
    let mut ord = 0u32;
    while at < bytes.len() {
        ord = ord
            .checked_add(u32::try_from(unvarint(bytes, &mut at)?)?)
            .context("an ordinal overflows")?;
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
        let b = *bytes.get(*at).context("a segment ends inside a number")?;
        *at += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    bail!("a number in a segment is too long")
}

/// A varint read from `r`.
fn read_varint(r: &mut impl Read) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        v |= u64::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            return Ok(v);
        }
    }
    bail!("a number in a segment is too long")
}

/// How many leading bytes `a` and `b` share.
fn shared(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// One word as the words area holds it, after `prev`: rebuilt into `prev`, with its run's length.
/// `bytes` is read from `at`.
fn next_word(bytes: &[u8], at: &mut usize, prev: &mut Vec<u8>) -> Result<u64> {
    let keep = unvarint(bytes, at)? as usize;
    let len = unvarint(bytes, at)? as usize;
    ensure!(
        keep <= prev.len(),
        "a word shares more than the one before it"
    );
    let rest = bytes
        .get(*at..*at + len)
        .context("a segment ends inside a word")?;
    prev.truncate(keep);
    prev.extend_from_slice(rest);
    *at += len;
    unvarint(bytes, at)
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
    blocks: BufWriter<File>,
    count: u64,
    last: Vec<u8>,
    buf: Vec<u8>,
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
            blocks: BufWriter::new(tempfile::tempfile_in(dir)?),
            count: 0,
            last: Vec::new(),
            buf: Vec::new(),
        })
    }

    /// Add `word` with its run of postings. Words must come in ascending order.
    pub fn add(&mut self, word: &str, run: &[u8]) -> Result<()> {
        let w = word.as_bytes();
        ensure!(
            self.count == 0 || self.last.as_slice() < w,
            "words must be added in order"
        );
        // A block's first word is whole, so a lookup can start reading at any block.
        let keep = if self.count.is_multiple_of(BLOCK) {
            self.blocks.write_all(&self.words_len.to_le_bytes())?;
            self.blocks.write_all(&self.pos.to_le_bytes())?;
            0
        } else {
            shared(&self.last, w)
        };
        self.buf.clear();
        varint(&mut self.buf, keep as u64);
        varint(&mut self.buf, (w.len() - keep) as u64);
        self.buf.extend_from_slice(&w[keep..]);
        varint(&mut self.buf, run.len() as u64);
        self.words.write_all(&self.buf)?;
        self.words_len += self.buf.len() as u64;
        self.out.write_all(run)?;
        self.pos += run.len() as u64;
        self.count += 1;
        self.last.clear();
        self.last.extend_from_slice(w);
        Ok(())
    }

    /// Write the words, the blocks and the footer, and put the segment in place. Returns its
    /// size.
    pub fn finish(self) -> Result<u64> {
        let Self {
            path,
            tmp,
            mut out,
            pos,
            words,
            blocks,
            count,
            ..
        } = self;
        let words_start = pos;
        let mut words = words.into_inner()?;
        words.seek(SeekFrom::Start(0))?;
        let words_len = std::io::copy(&mut words, &mut out)?;
        let blocks_start = words_start + words_len;
        // Block offsets were written relative to the words area; they are kept that way.
        let mut blocks = blocks.into_inner()?;
        blocks.seek(SeekFrom::Start(0))?;
        let blocks_len = std::io::copy(&mut blocks, &mut out)?;
        out.write_all(&count.to_le_bytes())?;
        out.write_all(&words_start.to_le_bytes())?;
        out.write_all(&blocks_start.to_le_bytes())?;
        out.write_all(MAGIC)?;
        let file = out.into_inner()?;
        file.sync_all()?;
        fs::rename(&tmp, &path).with_context(|| format!("put {} in place", path.display()))?;
        Ok(blocks_start + blocks_len + FOOTER)
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
    blocks_start: u64,
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
        let (count, words_start, blocks_start) = (at(0), at(8), at(16));
        ensure!(
            pos <= words_start
                && words_start <= blocks_start
                && blocks_start + count.div_ceil(BLOCK) * ENTRY + FOOTER == len,
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
            blocks_start,
        })
    }

    fn blocks(&self) -> u64 {
        self.count.div_ceil(BLOCK)
    }

    /// Block `i`'s words, as bytes, and where its first word's run starts.
    fn block(&self, i: u64) -> Result<(Vec<u8>, u64)> {
        let n = self.blocks();
        let e = self.bytes(
            self.blocks_start + i * ENTRY,
            if i + 1 < n { 2 * ENTRY } else { ENTRY },
        )?;
        let at = |j: usize| u64::from_le_bytes(e[j..j + 8].try_into().unwrap());
        let (start, run) = (at(0), at(8));
        let end = if i + 1 < n {
            at(16)
        } else {
            self.blocks_start - self.words_start
        };
        ensure!(start <= end, "{} does not add up", self.path.display());
        Ok((self.bytes(self.words_start + start, end - start)?, run))
    }

    /// The run of postings for `word`, if the segment has it.
    pub fn get(&self, word: &str) -> Result<Option<Vec<u8>>> {
        let w = word.as_bytes();
        // The last block whose first word is not after `word`.
        let (mut lo, mut hi) = (0u64, self.blocks());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (bytes, _) = self.block(mid)?;
            let mut first = Vec::new();
            next_word(&bytes, &mut 0, &mut first)?;
            if first.as_slice() <= w {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return Ok(None);
        }
        let (bytes, mut run) = self.block(lo - 1)?;
        let (mut at, mut cur) = (0, Vec::new());
        while at < bytes.len() {
            let len = next_word(&bytes, &mut at, &mut cur)?;
            match cur.as_slice().cmp(w) {
                std::cmp::Ordering::Less => run += len,
                std::cmp::Ordering::Equal => {
                    return Ok(Some(self.bytes(run, len)?));
                }
                std::cmp::Ordering::Greater => break,
            }
        }
        Ok(None)
    }

    /// Every word with its run of postings, in word order.
    pub fn words(&self) -> Result<Words> {
        let open = |at: u64, len: u64| -> Result<BufReader<std::io::Take<File>>> {
            let mut f = File::open(&self.path)?;
            f.seek(SeekFrom::Start(at))?;
            Ok(BufReader::new(f.take(len)))
        };
        Ok(Words {
            words: open(self.words_start, self.blocks_start - self.words_start)?,
            runs: open(self.postings_start, self.words_start - self.postings_start)?,
            left: self.count,
            prev: Vec::new(),
        })
    }

    fn bytes(&self, at: u64, len: u64) -> Result<Vec<u8>> {
        let mut out = vec![0u8; usize::try_from(len)?];
        (&self.file).seek(SeekFrom::Start(at))?;
        (&self.file).read_exact(&mut out)?;
        Ok(out)
    }
}

fn u32_at(r: &mut impl Read) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// A segment's words in order, read front to back.
pub struct Words {
    words: BufReader<std::io::Take<File>>,
    runs: BufReader<std::io::Take<File>>,
    left: u64,
    prev: Vec<u8>,
}

impl Iterator for Words {
    type Item = Result<(String, Vec<u8>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let mut read = || -> Result<(String, Vec<u8>)> {
            let keep = read_varint(&mut self.words)? as usize;
            let len = read_varint(&mut self.words)? as usize;
            ensure!(
                keep <= self.prev.len(),
                "a word shares more than the one before it"
            );
            self.prev.truncate(keep);
            let at = self.prev.len();
            self.prev.resize(at + len, 0);
            self.words.read_exact(&mut self.prev[at..])?;
            let run_len = read_varint(&mut self.words)? as usize;
            let mut run = vec![0u8; run_len];
            self.runs.read_exact(&mut run)?;
            Ok((String::from_utf8(self.prev.clone())?, run))
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
        // More words than a block holds, sharing beginnings, and one not ASCII.
        let mut words = vec![
            ("alpha", vec![p(0, 3, &[1, 4, 300])]),
            ("alphabet", vec![p(1, 1, &[2])]),
            ("alphabets", vec![p(0, 1, &[9]), p(1, 1, &[9])]),
            ("alpine", vec![p(1, 2, &[])]),
            ("beta", vec![p(0, 1, &[2]), p(1, 2, &[0, 7])]),
            ("betas", vec![p(0, 1, &[3])]),
            ("zed", vec![p(1, 1, &[1_000_000])]),
            ("zé", vec![p(0, 1, &[5])]),
            ("zéro", vec![p(0, 1, &[6]), p(1, 4, &[6, 8])]),
        ];
        words.sort_by(|a, b| a.0.cmp(b.0));
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
        for missing in [
            "",
            "a",
            "alph",
            "alphas",
            "alphabetz",
            "b",
            "c",
            "zf",
            "zzz",
        ] {
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

        // A segment with no words at all (sessions with no text) is a segment.
        Writer::create(&path, &["a".to_string()])
            .unwrap()
            .finish()
            .unwrap();
        let s = Segment::open(&path).unwrap();
        assert!(s.get("x").unwrap().is_none());
        assert_eq!(s.words().unwrap().count(), 0);
    }
}
