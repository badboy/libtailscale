use std::{
    io::{self, Read, Write},
    net::TcpStream,
    thread,
};

use tsnet::{ServerBuilder, Network};

fn main() {
    env_logger::init();
    let ts = ServerBuilder::new().ephemeral().redirect_log().build().unwrap();
    let ln = ts.listen(Network::Tcp, ":1999").unwrap();

    for conn in ln {
        match conn {
            Ok(conn) => {
                thread::spawn(move || {
                    handle_client(conn);
                });
            }
            Err(err) => panic!("{err}"),
        }
    }
}

fn handle_client(mut stream: TcpStream) {
    let mut buf = [0; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => {
                break;
            }
            Ok(n) => {
                io::stdout().write_all(&buf[0..n]).unwrap();
            }
            Err(err) => panic!("{err}"),
        }
    }
}
