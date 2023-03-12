use std::{env, io::Write};

use libtailscale::{ServerBuilder, Network};

fn main() {
    let target = env::args()
        .skip(1)
        .next()
        .expect("usage: echoclient host:port");
    let srv = ServerBuilder::new()
        .hostname("libtailscale-rs-echoclient")
        .ephemeral()
        .authkey(env::var("TS_AUTHKEY").expect("set TS_AUTHKEY in environment"))
        .build()
        .unwrap();

    let mut conn = srv.connect(Network::Tcp, &target).unwrap();
    write!(
        conn,
        "This is a test of the Tailscale connection service.\n"
    )
    .unwrap();
}
