extern crate cc;
use std::env;
use std::path::Path;

fn main() {
    let root_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = env::var("TARGET").unwrap();

    let parts = target.splitn(4, '-').collect::<Vec<_>>();
    let arch = parts[0];
    let sys = parts[2];

    if sys != "windows" {
        panic!("Platform '{}' not supported.", sys);
    }

    let hde = match arch {
        "i686" => "hde/hde32.c",
        "x86_64" => "hde/hde64.c",
        _ => panic!("Architecture '{}' not supported.", arch),
    };

    let mh_src_dir = Path::new(&root_dir).join("vendor/minhook/src");

    cc::Build::new()
        .file(mh_src_dir.join("buffer.c"))
        .file(mh_src_dir.join("hook.c"))
        .file(mh_src_dir.join("trampoline.c"))
        .file(mh_src_dir.join(hde))
        .compile("libminhook.a");

    println!("cargo:rerun-if-changed=vendor/minhook/src/");
    println!(
        "cargo:rustc-link-search=native={}",
        env::var("OUT_DIR").unwrap()
    );
}
