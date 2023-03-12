# tsnet - bindings for libtailscale.

libtailscale is a C library that embeds Tailscale into a process.
tsnet is a Rust crate wrapping libtailscale and exposing a Rust-y API on top.

Use this library to compile Tailscale into your program and get
an IP address on a tailnet, entirely from userspace.

## Requirements

* Rust compiler & Cargo
* Go v1.20 or higher

## Getting started

After running `cargo init` add the following lines to your `Cargo.toml` file:

```toml
tsnet = "0.1.0"
```

## Development

Build with

```
cargo build
```

Run tests with

```
cargo test
```

Run the examples with

```
cargo run --example echo_server
cargo run --example echo_client
```

## Bugs

Please file any issues about this code or the hosted service on
[the issue tracker](https://github.com/badboy/tailscale/issues).

## License

BSD 3-Clause for this repository, see LICENSE.
