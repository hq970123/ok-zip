//! Split volumes (`archive.ext.001`, `.002`, ...) as written by 7-Zip and
//! Bandizip: the archive byte stream is simply cut into fixed-size pieces.

use crate::error::{Error, Result};
use crate::progress::Progress;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Anything we can read archives from.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// If `path` looks like a numbered volume (`x.zip.001`), return the logical
/// archive path (`x.zip`) and the ordered list of existing volumes.
pub fn volume_set(path: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let dot = name.rfind('.')?;
    let num = &name[dot + 1..];
    if num.len() != 3 || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let base_name = &name[..dot];
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut vols = Vec::new();
    for i in 1..=999u32 {
        let p = dir.join(format!("{base_name}.{i:03}"));
        if p.is_file() {
            vols.push(p);
        } else {
            break;
        }
    }
    if vols.is_empty() {
        return None;
    }
    Some((dir.join(base_name), vols))
}

/// Read + Seek over several files as if they were concatenated.
pub struct MultiFileReader {
    files: Vec<File>,
    starts: Vec<u64>, // start offset of each file
    total: u64,
    pos: u64,
}

impl MultiFileReader {
    pub fn open(paths: &[PathBuf]) -> io::Result<Self> {
        let mut files = Vec::new();
        let mut starts = Vec::new();
        let mut total = 0;
        for p in paths {
            let f = File::open(p)?;
            let len = f.metadata()?.len();
            starts.push(total);
            total += len;
            files.push(f);
        }
        Ok(MultiFileReader { files, starts, total, pos: 0 })
    }

    pub fn len(&self) -> u64 {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    fn locate(&self, pos: u64) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(mut i) => {
                // Skip zero-length volumes.
                while i + 1 < self.starts.len() && self.starts[i + 1] == pos {
                    i += 1;
                }
                i
            }
            Err(i) => i.saturating_sub(1),
        }
    }
}

impl Read for MultiFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.total || buf.is_empty() || self.files.is_empty() {
            return Ok(0);
        }
        let idx = self.locate(self.pos);
        let off = self.pos - self.starts[idx];
        let end = if idx + 1 < self.starts.len() { self.starts[idx + 1] } else { self.total };
        let avail = (end - self.pos) as usize;
        let want = buf.len().min(avail);
        let f = &mut self.files[idx];
        f.seek(SeekFrom::Start(off))?;
        let n = f.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for MultiFileReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.total as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// Open an archive (or its volume set) for random access.
/// Returns the reader, total size and number of volumes.
pub fn open_reader(path: &Path) -> Result<(Box<dyn ReadSeek>, u64, usize)> {
    if let Some((_, vols)) = volume_set(path) {
        if vols.len() > 1 || path.extension().is_some_and(|e| e == "001") {
            let r = MultiFileReader::open(&vols)?;
            let len = r.len();
            let n = vols.len();
            return Ok((Box::new(BufReader::with_capacity(256 * 1024, r)), len, n));
        }
    }
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    Ok((Box::new(BufReader::with_capacity(256 * 1024, f)), len, 1))
}

/// Cut `src` into `<dest_base>.001`, `.002`, ... of at most `size` bytes,
/// then delete `src`. Returns the volume paths.
pub fn split_file(src: &Path, dest_base: &Path, size: u64, progress: &Progress) -> Result<Vec<PathBuf>> {
    if size == 0 {
        return Err(Error::Other("split size must be > 0".into()));
    }
    let mut input = BufReader::new(File::open(src)?);
    let total = fs::metadata(src)?.len();
    let mut out = Vec::new();
    let mut written = 0u64;
    let mut idx = 1u32;
    let mut buf = vec![0u8; 1024 * 1024];
    while written < total || idx == 1 {
        let name = format!(
            "{}.{idx:03}",
            dest_base.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        );
        let vol = dest_base.with_file_name(name);
        let mut w = BufWriter::new(File::create(&vol)?);
        let mut left = size.min(total - written);
        while left > 0 {
            progress.check()?;
            let want = buf.len().min(left as usize);
            input.read_exact(&mut buf[..want])?;
            w.write_all(&buf[..want])?;
            left -= want as u64;
            written += want as u64;
        }
        w.flush()?;
        out.push(vol);
        idx += 1;
        if idx > 999 {
            return Err(Error::Other("too many volumes (max 999)".into()));
        }
        if total == 0 {
            break;
        }
    }
    drop(input);
    fs::remove_file(src)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_reader_reads_across_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let src = dir.path().join("blob.bin");
        fs::write(&src, &data).unwrap();
        let vols = split_file(&src, &dir.path().join("blob.bin"), 3000, &Progress::new()).unwrap();
        assert_eq!(vols.len(), 4);
        let (_, set) = volume_set(&vols[2]).unwrap();
        assert_eq!(set.len(), 4);
        let mut r = MultiFileReader::open(&set).unwrap();
        let mut back = Vec::new();
        r.read_to_end(&mut back).unwrap();
        assert_eq!(back, data);
        r.seek(SeekFrom::Start(2995)).unwrap();
        let mut b = [0u8; 10];
        r.read_exact(&mut b).unwrap();
        assert_eq!(&b[..], &data[2995..3005]);
    }
}
