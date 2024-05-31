#![allow(dead_code)]

use std::{
    error::Error,
    path::{Path, PathBuf},
};

mod player;
mod rustplay;
mod templ;

use rustplay::RustPlay;

use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq)]
enum VisualizerPos {
    None,
    Right,
    Below,
}

// impl Display for Visualizer {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str(format!("{:?}", self).as_str())
//     }
// }

#[derive(Parser, Debug)]
#[command(version, about, author, long_about = None)]
struct Args {
    songs: Vec<PathBuf>,
    /// Min frequency to show in visualizer
    #[arg(long, default_value_t = 15)]
    min_freq: u32,

    #[arg(long, default_value_t = 4000)]
    /// Max frequency to show in visualizer
    max_freq: u32,

    /// Where to show the visualizer
    #[arg(long, short = 'o', default_value = "right")]
    visualizer: VisualizerPos,

    #[arg(long, short = 'd', default_value_t = 2)]
    // How much to divide FFT data
    fft_div: usize,

    #[arg(long, short = 'H', default_value_t = 5)]
    // Height of visualizer in characters
    visualizer_height: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut rust_play = RustPlay::new(&args);

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
