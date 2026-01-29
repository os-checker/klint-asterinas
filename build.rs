fn probe_sysroot() -> String {
    std::process::Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|x| x.trim().to_owned())
        .expect("failed to probe rust sysroot")
}

fn main() {
    // No need to rerun for other changes.
    println!("cargo::rerun-if-changed=build.rs");

    // Probe rustc sysroot. Although this is automatically added when using Cargo, the compiled
    // binary would be missing the necessary RPATH so it cannot run without using Cargo.
    let sysroot = probe_sysroot();
    println!("cargo::rustc-link-arg=-Wl,-rpath={sysroot}/lib");

    // If the RUSTDOC environment variable is just a plain "rustdoc", we want to discover the full path.
    // NB: this is the case when built from nix.
    let mut rustdoc = std::env::var("RUSTDOC").unwrap_or_default();
    if rustdoc.is_empty() || rustdoc == "rustdoc" {
        rustdoc = format!("{sysroot}/bin/rustdoc");
        assert!(
            std::fs::exists(&rustdoc).unwrap_or_default(),
            "Cannot find RUSTDOC. This is an unknown environment, please file a bug report."
        );
    }
    println!("cargo::rustc-env=RUSTDOC={rustdoc}");
}
