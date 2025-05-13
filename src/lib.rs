#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]


use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

pub mod player;
pub mod resampler;
pub mod rustplay;
pub mod templ;
pub mod term_extra;
pub mod value;

pub use rustplay::RustPlay;

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
pub enum VisualizerPos {
    #[default]
    None,
    Right,
    Below,
}

#[derive(Default, Parser, Debug, Clone)]
#[command(version, about, author, long_about = None)]
pub struct Args {
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

