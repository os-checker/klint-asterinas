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
}
