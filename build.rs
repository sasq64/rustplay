extern crate cmake;
use cmake::Config;

fn main() {
    let dst = Config::new("musicplayer").build();

    println!("cargo:rustc-link-search=native={}", dst.display());
    println!("cargo:rustc-link-lib=dylib=musix");
    //    println!("cargo:rustc-link-lib=dylib=asound");
}
