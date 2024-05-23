// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![feature(lazy_cell)]

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

static PROFILE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let current_exe_path = env::current_exe().unwrap();
    let deps_path = current_exe_path.parent().unwrap();
    let profile_path = deps_path.parent().unwrap();
    profile_path.into()
});

#[test]
fn run() {
    std::process::exit(
        std::process::Command::new("tests/dep/run.sh")
            .env("KLINT", PROFILE_PATH.join("klint"))
            .status()
            .unwrap()
            .code()
            .unwrap(),
    );
}
