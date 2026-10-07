use std::fs::{File, OpenOptions};
use std::io::SeekFrom;
use std::io::prelude::*;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::BackupMemoryInterface;
use rustboyadvance_utils::write_bin_file;

#[derive(Debug)]
pub struct BackupFile {
    size: usize,
    path: Option<PathBuf>,
    file: Option<File>,
    buffer: Vec<u8>,
    /// PATCH: set on every write, cleared by take_dirty()
    dirty: bool,
}

impl Clone for BackupFile {
    // PATCH: clone the in-memory buffer instead of re-reading from disk, so that
    // save_state()/thin_copy() keep the save data when no backing file is used.
    fn clone(&self) -> Self {
        BackupFile {
            size: self.size,
            path: self.path.clone(),
            file: None,
            buffer: self.buffer.clone(),
            dirty: self.dirty,
        }
    }
}

// PATCH: serialize the buffer contents (previously only size + path).
#[derive(Serialize, Deserialize)]
struct BackupFileRepr {
    size: usize,
    path: Option<PathBuf>,
    buffer: Vec<u8>,
}

impl Serialize for BackupFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        BackupFileRepr {
            size: self.size,
            path: self.path.clone(),
            buffer: self.buffer.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BackupFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let r = BackupFileRepr::deserialize(deserializer)?;
        let mut b = BackupFile::new(r.size, r.path);
        let n = r.buffer.len().min(b.buffer.len());
        b.buffer[..n].copy_from_slice(&r.buffer[..n]);
        Ok(b)
    }
}

impl BackupFile {
    pub fn new(size: usize, path: Option<PathBuf>) -> BackupFile {
        // TODO handle errors without unwrap
        let mut file: Option<File> = None;
        let buffer = if let Some(path) = &path {
            if !path.is_file() {
                write_bin_file(path, &vec![0xff; size]).unwrap();
            }

            let mut _file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap();

            let mut buffer = Vec::new();
            _file.read_to_end(&mut buffer).unwrap();
            buffer.resize(size, 0xff);

            file = Some(_file);

            buffer
        } else {
            vec![0xff; size]
        };

        BackupFile {
            size,
            path,
            file,
            buffer,
            dirty: false,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    /// PATCH: returns true if the buffer was written since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// PATCH: overwrite contents from external save bytes (truncated/padded with 0xff).
    pub fn load_bytes(&mut self, data: &[u8]) {
        let n = data.len().min(self.buffer.len());
        self.buffer[..n].copy_from_slice(&data[..n]);
        for b in &mut self.buffer[n..] {
            *b = 0xff;
        }
        self.dirty = false;
        self.flush();
    }

    pub fn flush(&mut self) {
        if let Some(file) = &mut self.file {
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&self.buffer).unwrap();
        }
    }
}

impl BackupMemoryInterface for BackupFile {
    fn write(&mut self, offset: usize, value: u8) {
        if self.buffer[offset] != value {
            self.dirty = true;
        }
        self.buffer[offset] = value;
        if let Some(file) = &mut self.file {
            file.seek(SeekFrom::Start(offset as u64)).unwrap();
            file.write_all(&[value]).unwrap();
        }
    }

    fn read(&self, offset: usize) -> u8 {
        self.buffer[offset]
    }

    fn resize(&mut self, new_size: usize) {
        self.size = new_size;
        self.buffer.resize(new_size, 0xff);
        self.flush();
    }
}
