#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

//! rustplay main file
use std::{
    cell::RefCell,
    collections::HashMap,
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
    panic,
    path::PathBuf,
    process,
    rc::Rc,
    sync::{LazyLock, Mutex},
    time::Duration,
};

mod player;
mod resampler;
mod rustplay;
mod templ;
mod term_extra;
mod value;

use rhai::FnPtr;
use rustplay::RustPlay;

use clap::{Parser, ValueEnum};

pub fn log(text: &str, file_name: &str, line: u32) {
    static LOG_FILE: LazyLock<Mutex<File>> = LazyLock::new(|| {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(".rustplay.log")
            .expect("Failed to open log file");
        Mutex::new(file)
    });
    let mut log_file = LOG_FILE.lock().unwrap();
    writeln!(log_file, "[{file_name}:{line}] {}", text).unwrap();
    log_file.flush().unwrap();
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        $crate::log(&format!($($arg)*), file!(), line!());
    }};
}

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

#[derive(Clone, Debug, Default)]
struct TemplateVar {
    color: Option<u32>,
    alias: Option<String>,
    code: Option<FnPtr>,
}

trait DynamicVar {
    fn generate(&self) -> String;
}

#[derive(Clone, Debug, Default)]
struct Settings {
    args: Args,
    template: String,
    width: i32,
    variables: HashMap<String, TemplateVar>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let orig_hook = panic::take_hook();
    // Settings is passed to RHAI and needs static lifetime => Rc
    // Needs to be modified by RHAI => RefCell (could be Cell)
    let settings = Rc::new(RefCell::new(Settings {
        args: Args::parse(),
        width: -1,
        ..Settings::default()
    }));

    let mut rhai_engine = rhai::Engine::new();

    rhai_engine.register_fn("template", {
        let settings = settings.clone();
        move |t: &str| t.clone_into(&mut settings.borrow_mut().template)
    });

    rhai_engine
        .register_fn("add_alias", {
            let settings = settings.clone();
            move |name: &str, alias: &str| {
                let v = TemplateVar {
                    alias: Some(alias.to_owned()),
                    ..TemplateVar::default()
                };
                settings.borrow_mut().variables.insert(name.to_owned(), v);
            }
        })
        .register_fn("add_alias", {
            let settings = settings.clone();
            move |name: &str, color: i64| {
                let v = TemplateVar {
                    color: Some(color as u32),
                    ..TemplateVar::default()
                };
                settings.borrow_mut().variables.insert(name.to_owned(), v);
            }
        })
        .register_fn("add_alias", {
            let settings = settings.clone();
            move |name: &str, alias: &str, color: i64| {
                let v = TemplateVar {
                    color: Some(color as u32),
                    alias: Some(alias.to_owned()),
                    ..TemplateVar::default()
                };
                settings.borrow_mut().variables.insert(name.to_owned(), v);
            }
        })
        .register_fn("add_alias", {
            let settings = settings.clone();
            move |name: &str, code: FnPtr| {
                let v = TemplateVar {
                    code: Some(code),
                    ..TemplateVar::default()
                };
                settings.borrow_mut().variables.insert(name.to_owned(), v);
            }
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
