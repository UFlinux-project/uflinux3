fn main() {
    println!("cargo:rerun-if-changed=src/renderer.c");
    println!("cargo:rerun-if-changed=src/renderer_ids.h");
    let vk = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("vulkan")
        .unwrap();
    let egl = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("egl")
        .unwrap();
    let gles = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("glesv2")
        .unwrap();
    cc::Build::new()
        .file("src/renderer.c")
        .includes(&vk.include_paths)
        .includes(&egl.include_paths)
        .includes(&gles.include_paths)
        .compile("uflinux_renderer_probe");
    for library in [vk, egl, gles] {
        for path in library.link_paths {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
        for name in library.libs {
            println!("cargo:rustc-link-lib={name}");
        }
    }
}
