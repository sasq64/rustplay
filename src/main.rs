use clap::Parser;
use std::{error::Error, panic, process, time::Duration};

use oldplay::Args;
use oldplay::RustPlay;

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
        rust_play.update();
        rust_play.draw_screen()?;
        std::thread::sleep(Duration::from_millis(5));
    }

    rust_play.quit()?;

    Ok(())
}
