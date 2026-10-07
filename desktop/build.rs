fn main() {
    println!("cargo:rerun-if-changed=src/pw_shim.c");
    let pw = pkg_config::Config::new()
        .atleast_version("0.3.49")
        .probe("libpipewire-0.3")
        .expect("libpipewire-0.3 development files (pkg-config libpipewire-0.3) are required");
    let mut build = cc::Build::new();
    build.file("src/pw_shim.c").flag_if_supported("-std=gnu11").warnings(false);
    for p in &pw.include_paths {
        build.include(p);
    }
    for (k, v) in &pw.defines {
        build.define(k, v.as_deref());
    }
    build.compile("vpm_pw_shim");
}
