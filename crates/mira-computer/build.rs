//! Generates the `cua.env.v1` gRPC client from the protos in `proto/`.
//!
//! Prefer a `protoc` on `PATH` — contributors usually already have one, and
//! it is usually newer than the vendored copy — but fall back to the binary in
//! `protoc-bin-vendored` so a fresh checkout and CI both build with no extra
//! setup.

use std::path::{Path, PathBuf};

/// The protos we compile, plus everything they import. See `proto/README.md`
/// for why this subset.
const PROTOS: &[&str] = &[
    "cua/env/v1/computer.proto",
    "cua/env/v1/system.proto",
    "cua/env/v1/windows.proto",
];

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let include = root.join("proto");

    for proto in PROTOS {
        println!("cargo:rerun-if-changed=proto/{proto}");
    }
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var_os("PROTOC").is_none() && !on_path("protoc") {
        match protoc_bin_vendored::protoc_bin_path() {
            Ok(bin) => std::env::set_var("PROTOC", bin),
            Err(e) => panic!(
                "no `protoc` on PATH and no vendored binary for this target ({e}). \
                 Install protobuf-compiler, or point PROTOC at it."
            ),
        }
    }

    let inputs: Vec<PathBuf> = PROTOS.iter().map(|p| include.join(p)).collect();
    tonic_prost_build::configure()
        // The server side exists only for `tests/cua_daemon.rs`, which serves
        // a stand-in daemon over real gRPC so the client can be tested
        // without one. `mira` never runs a server from this crate.
        .build_server(true)
        .build_client(true)
        .compile_protos(&inputs, &[include])
        .expect("compiling the cua.env.v1 protos");
}

fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .any(|p| Path::new(&p).is_file())
}
