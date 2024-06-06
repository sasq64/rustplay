#![allow(dead_code)]

use std::{
    cell::RefCell,
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
};

mod player;
mod rustplay;
mod templ;

use rustplay::RustPlay;

use clap::{Parser, ValueEnum};

use rhai::Engine;

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

#[derive(Parser, Debug, Clone)]
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

    #[arg(long, default_value_t = false)]
    no_term: bool,
}

#[derive(Clone)]
struct Settings {
    args: Args,
    template: String,
    width: i32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let s = Settings {
        args: Args::parse(),
        template: "".to_owned(),
        width: -1,
    };
    let settings = Rc::new(RefCell::new(s));

    let mut engine = Engine::new();

    let sclone = settings.clone();
    engine.register_fn("template", move |t: &str| {
        sclone.borrow_mut().template = t.to_owned()
    });

    let p = Path::new("init.rhai");
    if p.is_file() {
        engine.run_file(p.into())?;
    } else {
        let script = include_str!("../init.rhai");
        engine.run(script)?;
    }

    let ss = settings.clone().borrow().clone();
    let mut rust_play = RustPlay::new(&ss);

    if settings.borrow().args.songs.is_empty() {
        let p = Path::new("music.s3m");
        if p.is_file() {
            rust_play.add_song(p)?;
        }
    }

    for song in settings.borrow().args.songs.iter() {
        rust_play.add_song(song)?;
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
