//! TAP device abstraction for bridging WASM computers to the real network.
//!
//! A TAP device is a kernel virtual network interface at Layer 2.
//! We open `/dev/net/tun`, configure it in TAP mode (IFF_TAP | IFF_NO_PI),
//! and read/write raw ethernet frames.
//!
//! Linux-only. Requires root or CAP_NET_ADMIN.

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::io::{AsRawFd, RawFd};

    // ioctl constants for TUN/TAP
    const TUNSETIFF: libc::c_ulong = 0x400454ca;
    const IFF_TAP: libc::c_short = 0x0002;
    const IFF_NO_PI: libc::c_short = 0x1000;

    /// A TAP network device.
    pub struct TapDevice {
        file: File,
        name: String,
    }

    #[repr(C)]
    struct IfReq {
        ifr_name: [u8; 16],
        ifr_flags: libc::c_short,
        _padding: [u8; 22],
    }

    impl TapDevice {
        /// Open or create a TAP device with the given name.
        /// If name is empty, the kernel assigns one (usually "tap0").
        pub fn open(name: &str) -> io::Result<Self> {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/net/tun")?;

            let mut req = IfReq {
                ifr_name: [0u8; 16],
                ifr_flags: IFF_TAP | IFF_NO_PI,
                _padding: [0u8; 22],
            };

            // Copy device name
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(15);
            req.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

            // ioctl TUNSETIFF
            let fd = file.as_raw_fd();
            let ret = unsafe {
                libc::ioctl(fd, TUNSETIFF as libc::c_ulong, &mut req as *mut IfReq)
            };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }

            // Extract the actual device name
            let actual_name = {
                let end = req.ifr_name.iter().position(|&b| b == 0).unwrap_or(16);
                String::from_utf8_lossy(&req.ifr_name[..end]).to_string()
            };

            // Set non-blocking
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(TapDevice {
                file,
                name: actual_name,
            })
        }

        /// Get the device name.
        pub fn name(&self) -> &str {
            &self.name
        }

        /// Send a frame through the TAP device (into the host network stack).
        pub fn send_frame(&mut self, frame: &[u8]) -> io::Result<usize> {
            self.file.write(frame)
        }

        /// Non-blocking receive of a frame from the TAP device.
        /// Returns Ok(Some(len)) if a frame was read, Ok(None) if nothing available.
        pub fn recv_frame(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>> {
            match self.file.read(buf) {
                Ok(n) => Ok(Some(n)),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(e),
            }
        }

        /// Get the raw fd for poll/select.
        pub fn as_raw_fd(&self) -> RawFd {
            self.file.as_raw_fd()
        }

        /// Duplicate the TAP device by dup()'ing the file descriptor.
        /// Both handles share the same underlying TAP interface.
        pub fn try_clone(&self) -> io::Result<Self> {
            let new_fd = unsafe { libc::dup(self.file.as_raw_fd()) };
            if new_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let new_file = unsafe { File::from_raw_fd(new_fd) };
            Ok(TapDevice {
                file: new_file,
                name: self.name.clone(),
            })
        }
    }

    // Need FromRawFd
    use std::os::unix::io::FromRawFd;
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::io;

    /// Stub TAP device for non-Linux platforms.
    pub struct TapDevice;

    impl TapDevice {
        pub fn open(_name: &str) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TAP devices are only supported on Linux",
            ))
        }

        pub fn name(&self) -> &str { "" }
        pub fn send_frame(&mut self, _frame: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "not supported"))
        }
        pub fn recv_frame(&mut self, _buf: &mut [u8]) -> io::Result<Option<usize>> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "not supported"))
        }
        pub fn try_clone(&self) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "not supported"))
        }
    }
}

pub use imp::TapDevice;
