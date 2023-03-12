mod sys;

use std::{
    ffi::{c_int, CStr, CString},
    fmt::Display,
    fs::File,
    io::{Read, Write},
    mem::ManuallyDrop,
    os::fd::FromRawFd,
};

#[derive(Debug)]
pub struct Error(String);

#[derive(Debug)]
pub enum Network {
    Tcp,
    Udp,
}

impl Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Tcp => write!(f, "tcp"),
            Network::Udp => write!(f, "udp"),
        }
    }
}

pub struct Tailscale {
    /// a handle onto a Tailscale serve
    handle: sys::tailscale,
}

fn err(handle: c_int, code: c_int) -> Result<(), Error> {
    if code < 0 {
        unsafe {
            let mut errmsg: [i8; 256] = [0; 256];
            let res = sys::tailscale_errmsg(handle, &mut errmsg as *mut _, errmsg.len());
            if res != 0 {
                panic!("tailscale_errmsg returned {res}");
            }

            let slice = CStr::from_ptr(&errmsg as *const _);
            let errmsg = String::from_utf8_lossy(slice.to_bytes()).to_string();
            Err(Error(errmsg))
        }
    } else {
        Ok(())
    }
}

impl Tailscale {
    /// Creates a tailscale server object.
    ///
    /// No network connection is initialized until [`start`] is called.
    pub fn new() -> Self {
        unsafe {
            Tailscale {
                handle: sys::tailscale_new(),
            }
        }
    }

    /// Connects the server to the tailnet.
    ///
    /// Calling this function is optional as it will be called by the first use
    /// of `listen` or `dial` on a server.
    ///
    /// See also: `up`.
    pub fn start(&self) -> Result<(), Error> {
        unsafe { err(self.handle, sys::tailscale_start(self.handle)) }
    }

    pub fn set_ephermal(&self, ephemeral: bool) -> Result<(), Error> {
        unsafe {
            err(
                self.handle,
                sys::tailscale_set_ephemeral(self.handle, ephemeral as c_int),
            )
        }
    }

    pub fn up(&self) -> Result<(), Error> {
        unsafe { err(self.handle, sys::tailscale_up(self.handle)) }
    }

    pub fn listen(&self, network: Network, address: &str) -> Result<Listener, Error> {
        unsafe {
            let c_network = CString::new(format!("{}", network)).unwrap();
            let c_addr = CString::new(address).unwrap();
            let mut out = 0;
            let res = err(
                self.handle,
                sys::tailscale_listen(
                    self.handle,
                    c_network.as_ptr(),
                    c_addr.as_ptr(),
                    &mut out as *mut _,
                ),
            );

            res.map(|_| Listener {
                ts: self.handle,
                handle: out,
            })
        }
    }
}

impl Drop for Tailscale {
    fn drop(&mut self) {
        unsafe {
            sys::tailscale_close(self.handle);
        }
    }
}

pub struct Listener {
    ts: sys::tailscale,
    handle: sys::tailscale_listener,
}

impl Listener {
    pub fn accept(&self) -> Result<Stream, Error> {
        unsafe {
            let mut conn = 0;

            let res = err(self.ts, sys::tailscale_accept(self.handle, &mut conn as *mut _));
            res.map(|_| Stream { handle: conn })
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            sys::tailscale_listener_close(self.handle);
        }
    }
}

#[derive(Debug)]
pub struct Stream {
    handle: sys::tailscale_conn,
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            drop(File::from_raw_fd(self.handle));
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        unsafe {
            let mut fd = ManuallyDrop::new(File::from_raw_fd(self.handle));
            fd.write(buf)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        unsafe {
            let mut fd = ManuallyDrop::new(File::from_raw_fd(self.handle));
            fd.flush()
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        unsafe {
            let mut fd = ManuallyDrop::new(File::from_raw_fd(self.handle));
            fd.read(buf)
        }
    }
}
