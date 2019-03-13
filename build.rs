extern crate cmake;
use cmake::Config;
use std::io;
use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};

fn visit_dirs(dir: &Path, target : &mut Vec<PathBuf>) {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit_dirs(&path, target);
            } else {
                println!("{}", path.to_str().unwrap());
                let name : String = path.file_name().unwrap().to_str().unwrap().to_string();
                if name.ends_with(".a") {
                    println!("cargo:rustc-link-search=native={}", dir.to_str().unwrap());
                    let len = name.len();
                    println!("cargo:rustc-link-lib=static={}", name[3..len-2].to_string());
                } 


                target.push(path);
            }
        }
    }
}

fn main() {
    let dst = Config::new("musicplayer").build();

    let mut paths = Vec::<PathBuf>::new();
    visit_dirs(&dst, &mut paths);

    //println!("cargo:rustc-link-search=native={}", dst.display());
    //println!("cargo:rustc-link-lib=dylib=musix");
    println!("cargo:rustc-link-lib=dylib=c++");
    //    println!("cargo:rustc-link-lib=dylib=asound");
}
