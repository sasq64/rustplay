#![allow(dead_code)]

//! rustplay main file
use std::{cell::RefCell, error::Error, panic, path::PathBuf, process, rc::Rc, time::Duration};

mod player;
mod rustplay;
mod templ;
mod term_extra;
mod value;

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

    #[arg(long, short = 'c', default_value_t = false)]
    no_color: bool,
}

#[derive(Clone)]
struct Settings {
    args: Args,
    template: String,
    width: i32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let orig_hook = panic::take_hook();
    // Settings is passed to RHAI and needs static lifetime => Rc
    // Needs to be modified by RHAI => RefCell (could be Cell)
    let settings = Rc::new(RefCell::new(Settings {
        args: Args::parse(),
        template: String::new(),
        width: -1,
    }));

    let mut rhai_engine = rhai::Engine::new();

    rhai_engine.register_fn("template", {
        let settings = settings.clone();
        move |t: &str| t.clone_into(&mut settings.borrow_mut().template)
    });

    let p = PathBuf::from("init.rhai");
    if p.is_file() {
        rhai_engine.run_file(p)?;
    } else {
        let script = include_str!("../init.rhai");
        rhai_engine.run(script)?;
    }

    let mut rust_play = RustPlay::new(settings.borrow().clone())?;

    panic::set_hook(Box::new(move |panic_info| {
        RustPlay::restore_term().expect("Could not restore terminal");
        println!("panic occurred: {panic_info}");
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        process::exit(1);
    }));

    if settings.borrow().args.songs.is_empty() {
        let test_song: PathBuf = "music.mod".into();
        if test_song.is_file() {
            settings.borrow_mut().args.songs.push(test_song);
        }
    }

    for song in &settings.borrow().args.songs {
        rust_play.add_path(song)?;
    }

    loop {
        let do_quit = rust_play.handle_keys()?;
        if do_quit {
            break;
        }
        rust_play.update_meta();
        rust_play.draw_screen()?;
        std::thread::sleep(Duration::from_millis(10));
    }

    rust_play.quit()?;

    Ok(())
}
