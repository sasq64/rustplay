#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

//! rustplay main file
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
    panic,
    path::PathBuf,
    process,
    sync::{LazyLock, Mutex},
    time::Duration,
};

mod player;
mod resampler;
mod rustplay;
mod templ;
mod term_extra;
mod value;

use rustplay::RustPlay;

use clap::{Parser, ValueEnum};

/// Log text to the '.rustplay.log' file
///
/// # Panics
///
/// Will panic if the mutex can not be locked (can not happen) or if
/// the log file can not be written to.
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
    writeln!(log_file, "[{file_name}:{line}] {text}").unwrap();
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

fn main() -> Result<(), Box<dyn Error>> {
    let orig_hook = panic::take_hook();
    let args = Args::parse();

    let mut rust_play = RustPlay::new(args)?;

    panic::set_hook(Box::new(move |panic_info| {
        RustPlay::restore_term().expect("Could not restore terminal");
        println!("panic occurred: {panic_info}");
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        process::exit(1);
    }));

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
