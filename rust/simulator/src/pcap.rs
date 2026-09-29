//! Minimal libpcap (classic .pcap, microsecond timestamps, LINKTYPE_ETHERNET)
//! writer. Files open in Wireshark/tcpdump.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct PcapWriter {
    out: BufWriter<File>,
    pub path: PathBuf,
    pub records: u64,
}

impl PcapWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&0xa1b2_c3d4u32.to_le_bytes())?; // magic, µs resolution
        out.write_all(&2u16.to_le_bytes())?; // version major
        out.write_all(&4u16.to_le_bytes())?; // version minor
        out.write_all(&0i32.to_le_bytes())?; // thiszone
        out.write_all(&0u32.to_le_bytes())?; // sigfigs
        out.write_all(&65535u32.to_le_bytes())?; // snaplen
        out.write_all(&1u32.to_le_bytes())?; // LINKTYPE_ETHERNET
        out.flush()?;
        Ok(PcapWriter { out, path: path.to_path_buf(), records: 0 })
    }

    /// `ts_ms`: absolute simulation time in milliseconds.
    pub fn write(&mut self, ts_ms: i64, frame: &[u8]) {
        let secs = ts_ms.div_euclid(1000) as u32;
        let usecs = (ts_ms.rem_euclid(1000) * 1000) as u32;
        let len = frame.len() as u32;
        let _ = self.out.write_all(&secs.to_le_bytes());
        let _ = self.out.write_all(&usecs.to_le_bytes());
        let _ = self.out.write_all(&len.to_le_bytes());
        let _ = self.out.write_all(&len.to_le_bytes());
        let _ = self.out.write_all(frame);
        self.records += 1;
        // Keep the file readable even if the run aborts.
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_header_and_record() {
        let dir = std::env::temp_dir().join(format!("ecm-pcap-test-{}", std::process::id()));
        let p = dir.join("t.pcap");
        {
            let mut w = PcapWriter::create(&p).unwrap();
            w.write(1_500, &[0xffu8; 60]);
        }
        let b = std::fs::read(&p).unwrap();
        assert_eq!(b.len(), 24 + 16 + 60);
        assert_eq!(&b[0..4], &0xa1b2_c3d4u32.to_le_bytes());
        assert_eq!(u32::from_le_bytes([b[20], b[21], b[22], b[23]]), 1);
        assert_eq!(u32::from_le_bytes([b[24], b[25], b[26], b[27]]), 1); // 1 s
        assert_eq!(u32::from_le_bytes([b[28], b[29], b[30], b[31]]), 500_000); // .5 s
        let _ = std::fs::remove_dir_all(dir);
    }
}
