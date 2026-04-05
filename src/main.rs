use anyhow::Context;
use clap::Parser;
use oldplay::CONFIG_LUA;
use std::{error::Error, panic, process, time::Duration};

use oldplay::Args;
use oldplay::RustPlay;

use anyhow::Result;

fn main() -> Result<(), Box<dyn Error>> {
    let orig_hook = panic::take_hook();
    let args = Args::parse();

    if args.write_config {
        let config_dir = dirs::config_dir().context("No config dir available")?;
        let out_path = config_dir.join("oldplay");
        std::fs::create_dir_all(&out_path)?;
        std::fs::write(out_path.join("config.lua"), CONFIG_LUA)?;
        std::process::exit(0);
    }

    panic::set_hook(Box::new(move |panic_info| {
        RustPlay::restore_term().expect("Could not restore terminal");
        println!("panic occurred: {panic_info}");
        // invoke the default handler and exit the process
        orig_hook(panic_info);
        process::exit(1);
    }));

    if let Err(e) = run_rustplay(args) {
        RustPlay::restore_term().expect("Could not restore terminal");
        eprintln!("Error: {e}");
    }
    Ok(())
}

fn run_rustplay(args: Args) -> Result<()> {
    let mut rust_play = RustPlay::new(args)?;
    loop {
        let do_quit = rust_play.handle_events()?;
        if do_quit {
            break;
        }
        rust_play.update()?;
        rust_play.draw_screen()?;
        std::thread::sleep(Duration::from_millis(5));
    }

    rust_play.destroy()?;

    Ok(())
}
