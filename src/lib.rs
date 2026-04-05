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

pub mod media_keys;
pub mod player;
pub mod resampler;
pub mod rustplay;
pub mod templ;
pub mod term_extra;
pub mod utils;
pub mod value;

pub use rustplay::RustPlay;

use clap::{Parser, ValueEnum};
use serde::Deserialize;

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

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct FFtSettings {
    min_freq: u32,
    max_freq: u32,
    visualizer_height: usize,
    bar_width: usize,
    bar_gap: usize,
    bar_count: usize,
    hann: bool,
    normalize: bool,
    colors: Vec<u32>,
}
impl Default for FFtSettings {
    fn default() -> Self {
        Self {
            min_freq: 40,
            max_freq: 12_1000,
            visualizer_height: 5,
            bar_width: 2,
            bar_gap: 1,
            bar_count: 25,
            hann: true,
            normalize: false,
            colors: vec![0xff0040, 0x00ff40],
        }
    }
}

#[derive(Default, Debug, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    fft: FFtSettings,
    no_color: bool,
}

#[derive(Default, Parser, Debug, Clone)]
#[command(version, about, author, long_about = None)]
pub struct Args {
    songs: Vec<PathBuf>,

    #[arg(long, default_value_t = false)]
    pub write_config: bool,

    #[arg(long, default_value_t = false)]
    no_term: bool,

    #[arg(long, short = 'c', default_value_t = false)]
    no_color: bool,
}

pub const CONFIG_LUA: &str = include_str!("../config.lua");
