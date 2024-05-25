#![allow(dead_code)]

use std::{
    error::Error,
    path::{Path, PathBuf},
};

mod player;
mod rustplay;

use rustplay::RustPlay;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    songs: Vec<PathBuf>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut rust_play = RustPlay::new();

    if args.songs.is_empty() {
        let p = Path::new("music.s3m");
        if p.is_file() {
            rust_play.add_song(p);
        }
    }

    for song in args.songs {
        rust_play.add_song(&song);
    }

    loop {
        let do_quit = rust_play.handle_keys()?;
        if do_quit {
            break;
        }
        rust_play.update_meta();
        rust_play.draw_screen()?;
    }

    rust_play.quit()?;

    Ok(())
}
