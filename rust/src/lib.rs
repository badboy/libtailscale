mod sys;

use std::{
    ffi::{c_int, CStr, CString},
    fmt::Display,
    fs::File,
    io::Read,
    net::TcpStream,
    os::fd::FromRawFd,
    path::PathBuf,
    thread,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("can't convert this from an OsString to a String, invalid unicode?")]
    CantConvertToString,

    #[error("tsnet: {0}")]
    TSNet(String),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("your string has NULL in it: {0}")]
    NullInString(#[from] std::ffi::NulError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

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

pub struct Server {
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
            Err(Error::TSNet(errmsg))
        }
    } else {
        Ok(())
    }
}

impl Server {
    pub fn connect(&self, network: Network, addr: &str) -> Result<TcpStream> {
        let mut conn: sys::tailscale_conn = 0;
        let network = CString::new(format!("{}", network)).unwrap();
        let addr = CString::new(addr)?;

        unsafe {
            err(
                self.handle,
                sys::tailscale_dial(self.handle, network.as_ptr(), addr.as_ptr(), &mut conn),
            )?
        }

        let conn = conn as c_int;
        Ok(unsafe { TcpStream::from_raw_fd(conn) })
    }

    pub fn listen(&self, network: Network, address: &str) -> Result<Listener, Error> {
        unsafe {
            let network = CString::new(format!("{}", network)).unwrap();
            let addr = CString::new(address).unwrap();
            let mut out = 0;
            let res = err(
                self.handle,
                sys::tailscale_listen(
                    self.handle,
                    network.as_ptr(),
                    addr.as_ptr(),
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

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            sys::tailscale_close(self.handle);
        }
    }
}

#[derive(Default)]
pub struct ServerBuilder {
    dir: Option<PathBuf>,
    hostname: Option<String>,
    authkey: Option<String>,
    control_url: Option<String>,
    ephemeral: bool,
    log: u8, // 0 = no change, 1 = redirect to `log`, 2 = disable
}

impl ServerBuilder {
    /// Creates a server builder.
    ///
    /// Call [`ServerBuilder::build`] to start the server.
    pub fn new() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub fn dir(mut self, dir: PathBuf) -> Self {
        self.dir = Some(dir);
        self
    }

    pub fn hostname(mut self, hostname: &str) -> Self {
        self.hostname = Some(hostname.to_owned());
        self
    }

    pub fn authkey(mut self, authkey: String) -> Self {
        self.authkey = Some(authkey);
        self
    }

    pub fn control_url(mut self, control_url: String) -> Self {
        self.control_url = Some(control_url);
        self
    }

    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    pub fn redirect_log(mut self) -> Self {
        self.log = 1;
        self
    }

    pub fn disable_log(mut self) -> Self {
        self.log = 2;
        self
    }

    pub fn build(self) -> Result<Server> {
        let result = unsafe {
            Server {
                handle: sys::tailscale_new(),
            }
        };

        match self.log {
            1 => {
                let (rx, wx) = nix::unistd::pipe().unwrap();
                let _ = thread::Builder::new()
                    .name("libtailscale-logwriter".to_string())
                    .spawn(move || {
                        let mut buf = [0; 2048];
                        let mut file = unsafe { File::from_raw_fd(rx) };
                        loop {
                            match file.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    let log = String::from_utf8_lossy(&buf[0..n]);
                                    log::info!("{}", log);
                                }
                                Err(err) => {
                                    log::error!("failed to read from log pipe: {err}");
                                    break;
                                }
                            }
                        }
                    });
                unsafe { err(result.handle, sys::tailscale_set_logfd(result.handle, wx))? }
            }
            2 => unsafe { err(result.handle, sys::tailscale_set_logfd(result.handle, -1))? },
            _ => {}
        }

        if let Some(dir) = self.dir {
            let dir = dir.into_os_string();
            let dir = dir.into_string().map_err(|_| Error::CantConvertToString)?;
            let dir = CString::new(dir)?;
            unsafe {
                err(
                    result.handle,
                    sys::tailscale_set_dir(result.handle, dir.as_ptr()),
                )?
            }
        }

        if let Some(hostname) = self.hostname {
            let hostname = CString::new(hostname)?;
            unsafe {
                err(
                    result.handle,
                    sys::tailscale_set_hostname(result.handle, hostname.as_ptr()),
                )?
            }
        }

        if let Some(authkey) = self.authkey {
            let authkey = CString::new(authkey)?;
            unsafe {
                err(
                    result.handle,
                    sys::tailscale_set_authkey(result.handle, authkey.as_ptr()),
                )?
            }
        }

        if let Some(control_url) = self.control_url {
            let control_url = CString::new(control_url)?;
            unsafe {
                err(
                    result.handle,
                    sys::tailscale_set_control_url(result.handle, control_url.as_ptr()),
                )?
            }
        }

        unsafe {
            err(
                result.handle,
                sys::tailscale_set_ephemeral(result.handle, if self.ephemeral { 1 } else { 0 }),
            )?
        }

        unsafe { err(result.handle, sys::tailscale_start(result.handle))? }

        Ok(result)
    }
}

pub struct Listener {
    ts: sys::tailscale,
    handle: sys::tailscale_listener,
}

impl Listener {
    pub fn accept(&self) -> Result<TcpStream, Error> {
        unsafe {
            let mut conn = 0;

            let res = err(
                self.ts,
                sys::tailscale_accept(self.handle, &mut conn as *mut _),
            );
            res.map(|_| TcpStream::from_raw_fd(conn))
        }
    }

    pub fn incoming(&mut self) -> &Self {
        self
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            sys::tailscale_listener_close(self.handle);
        }
    }
}

impl Iterator for Listener {
    type Item = Result<TcpStream>;
    fn next(&mut self) -> Option<Result<TcpStream>> {
        Some(self.accept())
    }
}
