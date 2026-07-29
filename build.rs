//! Records the **target triple** the binary was built for, so `--version` can report it
//! (M10).
//!
//! `TARGET` is set by cargo for build scripts only — there is no way to read it from
//! ordinary code, and reconstructing it from `std::env::consts` + `cfg!` would guess the
//! vendor field. The whole point of `--version` is that a downloaded executable can say
//! what it is *without* the repo next to it, so the value it prints must come from the
//! build itself and must not be able to lie.
//!
//! Deliberately the only thing this script does: no codegen, no dependencies, no network.

fn main() {
    let target = std::env::var("TARGET").expect("cargo always sets TARGET for a build script");
    println!("cargo:rustc-env=BUILD_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
