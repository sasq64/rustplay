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

#[derive(Default, ValueEnum, Clone, Copy, Debug, PartialEq)]
enum VisualizerPos {
    #[default]
    None,
    Right,
    Below,
}

// impl Display for Visualizer {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str(format!("{:?}", self).as_str())
//     }
// }

#[derive(Default, Parser, Debug, Clone)]
#[command(version, about, author, long_about = None)]
struct Args {
    songs: Vec<PathBuf>,

    #[arg(long, default_value_t = 15)]
    /// Min frequency to show in visualizer
    min_freq: u32,

    #[arg(long, default_value_t = 4000)]
    /// Max frequency to show in visualizer
    max_freq: u32,

    #[arg(long, short = 'o', default_value = "below")]
    /// Where to show the visualizer
    visualizer: VisualizerPos,

    #[arg(long, short = 'd', default_value_t = 4)]
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
    // Settings is passed to RHAI and needs static lifetime => Rc
    // Needs to be modified by RHAI => RefCell (could be Cell)
    let settings = Rc::new(RefCell::new(Settings {
        args: Args::parse(),
        template: "".to_owned(),
        width: -1,
    }));

    let mut rhai_engine = rhai::Engine::new();

    rhai_engine.register_fn("template", {
        let settings = settings.clone();
        move |t: &str| settings.borrow_mut().template = t.to_owned()
    });

    let p = Path::new("init.rhai");
    if p.is_file() {
        rhai_engine.run_file(p.into())?;
    } else {
        let script = include_str!("../init.rhai");
        rhai_engine.run(script)?;
    }

    let mut rust_play = RustPlay::new(settings.borrow().clone())?;

    if settings.borrow().args.songs.is_empty() {
        settings
            .borrow_mut()
            .args
            .songs
            .push(PathBuf::from("../musicplayer/music"));
    }

    for song in &settings.borrow().args.songs {
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
