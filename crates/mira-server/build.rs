//! Build script.
//!
//! `include_dir!("frontend/dist")` in `embedded.rs` needs the directory to
//! exist at compile time, even if empty. Cold-cloning the repo without a
//! prior `npm run build` in `crates/mira-server/frontend/` would otherwise
//! panic at macro expansion. Creating an empty dir here means `cargo build`
//! always succeeds; the runtime just serves the placeholder page until a
//! real dist/ is built.
//!
//! Rebuilds are triggered by the presence of files inside `frontend/dist`,
//! so a fresh `npm run build` invalidates the cached embed.

use std::fs;
use std::path::Path;

fn main() {
    let dist = Path::new("frontend").join("dist");
    if !dist.exists() {
        if let Err(e) = fs::create_dir_all(&dist) {
            println!(
                "cargo:warning=mira-server: could not create {}: {}",
                dist.display(),
                e
            );
        }
    }
    println!("cargo:rerun-if-changed=frontend/dist");
    println!("cargo:rerun-if-changed=build.rs");
}
