use std::io::{Read, self, Write};

use libtailscale::*;

fn main() {
    let ts = Tailscale::new();
    ts.set_ephermal(true).unwrap();
    ts.up().unwrap();
    let ln = ts.listen(Network::Tcp, ":1999").unwrap();

    loop {
        let mut conn = ln.accept().unwrap();
        let mut buf = [0; 2048];
        let nread = conn.read(&mut buf).unwrap();
        if nread > 0 {
            io::stdout().write_all(&buf[0..nread]).unwrap();
        }
    }

}
