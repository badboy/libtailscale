//! Compile Tailscale into your program and get an entirely userspace IP address on a tailnet.
//!
//! From here you can listen for other programs on your tailnet dialing you,
//! or connect directly to other services.
//!
//! Based on [`libtailscale`](https://github.com/tailscale/libtailscale), the C wrapper around the
//! Tailscale Go package.
//! See <https://pkg.go.dev/tailscale.com> for Go module docs.
//!
//! ## Examples
//!
//! ### Server
//!
//! ```rust,no_run
//! use std::net::TcpStream;
//! use libtailscale::{ServerBuilder, Network};
//!
//! fn main() {
//!     let ts = ServerBuilder::new().ephemeral().redirect_log().build().unwrap();
//!     let ln = ts.listen(Network::Tcp, ":1999").unwrap();
//!
//!     for conn in ln {
//!         match conn {
//!             Ok(conn) => handle_client(conn),
//!             Err(err) => panic!("{err}"),
//!         }
//!     }
//! }
//!
//! fn handle_client(mut stream: TcpStream) {
//!   // ...
//! }
//! ```
//!
//! ### Client
//!
//! ```rust,no_run
//! use std::{env, io::Write};
//!
//! use libtailscale::{ServerBuilder, Network};
//!
//! fn main() {
//!     let srv = ServerBuilder::new()
//!         .ephemeral()
//!         .build()
//!         .unwrap();
//!
//!     let mut conn = srv.connect(Network::Tcp, "echo-server:1999").unwrap();
//!     write!(conn, "This is a test of the Tailscale connection service.\n").unwrap();
//! }
//! ```
#[deny(missing_docs)]
#[allow(non_camel_case_types, dead_code)]
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
#[cfg(feature = "tokio")]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[cfg(feature = "tokio")]
use hyper::server::accept::Accept;
#[cfg(feature = "tokio")]
use tokio::{net, task};

/// Possible errors
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Data passed in cannot be converted to a string, possibly invalid unicode.
    #[error("can't convert this from an OsString to a String, invalid unicode?")]
    CantConvertToString,

    /// Errors from the underlying `libtailscale`
    #[error("tsnet: {0}")]
    TSNet(String),

    /// IO errors from network handles
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    /// Data passed in contained NULL bytes.
    #[error("your string has NULL in it: {0}")]
    NullInString(#[from] std::ffi::NulError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The possible network types.
#[derive(Debug)]
pub enum Network {
    /// TCP
    Tcp,
    /// UDP.
    ///
    /// Note: UDP currently not really tested.
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

/// A Tailscale node.
///
/// Can act as a server or client.
///
/// ## Example
///
/// ```rust,no_run
/// use libtailscale::{ServerBuilder, Network};
///
/// let server = ServerBuilder::new().ephemeral().build().unwrap();
/// let ln = server.listen(Network::Tcp, ":1999").unwrap();
///
/// for conn in ln {
///   // handle connection
/// }
/// ```
pub struct Server {
    /// a handle onto a Tailscale server
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
    /// Connect to the given address over the specified network.
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

    /// Listen on the given address and network for new connections.
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

    #[cfg(feature = "tokio")]
    pub fn listen_async(&self, network: Network, address: &str) -> Result<AsyncListener, Error> {
        let ls = self.listen(network, address)?;
        Ok(AsyncListener {
            listener: ls,
            fut: None,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            sys::tailscale_close(self.handle);
        }
    }
}

/// Server factory, which can be used to configure the properties of a new server.
///
/// Methods can be chained on it in order to configure it.
///
/// ## Example
///
/// ```rust,no_run
/// use libtailscale::ServerBuilder;
///
/// let server = ServerBuilder::new().ephemeral().build().unwrap();
/// ```
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
    /// Generates the base configuration for a new server, from which configuration methods can be chained.
    ///
    /// Call [`ServerBuilder::build`] to start the server.
    pub fn new() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Specifies the name of the directory to use for state.
    /// If unset, a directory is selected automatically under the user's configuration directory
    /// (see <https://golang.org/pkg/os/#UserConfigDir>), based on the name of the binary.
    pub fn dir(mut self, dir: PathBuf) -> Self {
        self.dir = Some(dir);
        self
    }

    /// The hostname to present to the control server.
    /// If unset, the binary name is used.
    pub fn hostname(mut self, hostname: &str) -> Self {
        self.hostname = Some(hostname.to_owned());
        self
    }

    /// AuthKey, if set, is the auth key to create the node
    /// and will be preferred over the `TS_AUTHKEY` environment variable.
    /// If the node is already created (from state previously stored in in Store),
    /// then this field is not used.
    pub fn authkey(mut self, authkey: String) -> Self {
        self.authkey = Some(authkey);
        self
    }

    /// Specifies the coordination server URL.
    /// If unset, the Tailscale default is used.
    pub fn control_url(mut self, control_url: String) -> Self {
        self.control_url = Some(control_url);
        self
    }

    /// Specifies that the instance should register as an Ephemeral node
    /// (see <https://tailscale.com/kb/1111/ephemeral-nodes/>)
    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    /// Redirect `libtailscale` logging to `log`.
    ///
    /// * This starts a new thread to handle logs.
    /// * Everything from `libtailscale` is logged at `INFO` level.
    /// * Use an appropriate logger to handle log output further.
    pub fn redirect_log(mut self) -> Self {
        self.log = 1;
        self
    }

    /// Disable `libtailscale` logging.
    ///
    /// See also [`ServerBuilder::redirect_log`] to rely on the Rust `log` facade.
    pub fn disable_log(mut self) -> Self {
        self.log = 2;
        self
    }

    /// Start the server using the configured options.
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

/// A server, listening for connections.
///
/// After creating a server by binding it to a socket address,
/// it listens for incoming connections.
/// These can be accepted by calling accept or by iterating over the Incoming iterator returned by incoming.
///
/// ## Examples
///
/// ```rust,no_run
/// use std::net::TcpStream;
/// use libtailscale::{ServerBuilder, Network, Result};
///
/// fn handle_client(stream: TcpStream) {
///     // ...
/// }
///
/// fn main() -> Result<()> {
///   let ts = ServerBuilder::new().ephemeral().redirect_log().build().unwrap();
///   let mut listener = ts.listen(Network::Tcp, ":1999").unwrap();
///
///   // accept connections and process them serially
///   for stream in listener.incoming() {
///         handle_client(stream?);
///   }
///   Ok(())
/// }
/// ```
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

impl Iterator for &Listener {
    type Item = Result<TcpStream>;
    fn next(&mut self) -> Option<Result<TcpStream>> {
        Some(self.accept())
    }
}

#[cfg(feature = "tokio")]
pub struct AsyncListener {
    listener: Listener,
    fut: Option<task::JoinHandle<Result<net::TcpStream>>>,
}

#[cfg(feature = "tokio")]
impl Accept for AsyncListener {
    type Conn = net::TcpStream;
    type Error = Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        if self.fut.is_none() {
            let ts = self.listener.handle;
            self.fut = Some(task::spawn_blocking(move || unsafe {
                let mut conn = 0;

                let res = err(ts, sys::tailscale_accept(ts, &mut conn as *mut _));
                res.map(|_| {
                    let stream = TcpStream::from_raw_fd(conn);
                    let _ = stream.set_nonblocking(true);
                    net::TcpStream::from_std(stream).unwrap()
                })
            }));
        }

        let mut fut = self.fut.take().unwrap();
        let fut_pl = Pin::new(&mut fut);
        let stream = match fut_pl.poll(cx) {
            Poll::Ready(t) => t,
            Poll::Pending => {
                self.fut = Some(fut);
                return Poll::Pending;
            }
        };

        Poll::Ready(Some(stream.unwrap()))
    }
}
