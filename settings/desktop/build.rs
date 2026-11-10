fn main() {
    println!("cargo:rerun-if-changed=../ui/src/polkit-agent.c");
    let pkg = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("polkit-agent-1")
        .unwrap();
    cc::Build::new()
        .file("../ui/src/polkit-agent.c")
        .includes(pkg.include_paths)
        .compile("uflinux_polkit_agent");
    for path in pkg.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    for name in pkg.libs {
        println!("cargo:rustc-link-lib={name}");
    }
}
