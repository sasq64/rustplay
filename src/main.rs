use cpal::{traits::*, SupportedBufferSize};
use musix::ChipPlayer;
use ringbuf::{traits::*, StaticRb};
use std::io::stdout;
use std::{error::Error, path::Path, thread};

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};

use ratatui::{prelude::*, widgets::*};
type Cmd = Box<dyn FnOnce(&mut ChipPlayer) -> Option<ChipPlayer> + Send>;
type Info = (String, String);

fn run_player<P, C>(mut info_producer: P, mut cmd_consumer: C) -> Result<(), Box<dyn Error>>
where
    P: Producer<Item = Info> + Send + 'static,
    C: Consumer<Item = Cmd> + Send + 'static,
{
    let device = cpal::default_host().default_output_device().unwrap();
    let mut config = device.default_output_config()?;
    let configs = device.supported_output_configs()?;
    let mut buffer_size = 0;
    for conf in configs {
        if let SupportedBufferSize::Range { max: b, .. } = conf.buffer_size() {
            if *b > buffer_size {
                buffer_size = *b;
            }
        }
        if let Some(conf2) = conf.try_with_sample_rate(cpal::SampleRate(44100)) {
            config = conf2;
            break;
        }
    }

    thread::spawn(move || {
        let ring = StaticRb::<f32, 32768>::default();
        let (mut producer, mut consumer) = ring.split();

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumer.pop_slice(data);
                },
                |err| eprintln!("an error occurred on stream: {err}"),
                None,
            )
            .unwrap();
        stream.play().unwrap();
        let mut target: Vec<i16> = vec![0; buffer_size as usize];

        //let name = &args[1];
        //let mut player = musix::load_song(&name).unwrap();
        let mut player = musix::ChipPlayer::new();

        loop {
            if let Some(f) = cmd_consumer.try_pop() {
                if let Some(new_player) = f(&mut player) {
                    player = new_player;
                }
            }

            while let Some(meta) = player.get_changed_meta() {
                let val = player.get_meta_string(&meta).unwrap();
                let _ = info_producer.try_push((meta, val));
            }
            if producer.vacant_len() > target.len() {
                player.get_samples(&mut target);
                producer.push_iter(target.iter().map(|i| (*i as f32) / 32767.0));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    });
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("No song to play!");
        return Ok(());
    }

    musix::init(Path::new("data"))?;

    let cmd_buf = StaticRb::<Cmd, 32>::default();
    let (mut cmd_producer, cmd_consumer) = cmd_buf.split();

    let info_buf = StaticRb::<Info, 64>::default();
    let (info_producer, mut info_consumer) = info_buf.split();

    let _ = run_player(info_producer, cmd_consumer);

    enable_raw_mode().unwrap();
    stdout().execute(EnterAlternateScreen).unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();

    let _ = cmd_producer.try_push(Box::new(move |_| {
        //p.seek(2, -1);
        musix::load_song(&args[1]).ok()
    }));

    let ms = std::time::Duration::from_millis(50);
    loop {
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new("X")
                        .block(Block::default().title("Play Music").borders(Borders::ALL)),
                    frame.size(),
                )
            })
            .unwrap();
        if event::poll(ms).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == event::KeyEventKind::Press {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    //if key.code == KeyCode::Char('n') || key.code == KeyCode::Right {
                    //    music_player.next_song();
                    //}
                    //if key.code == KeyCode::Char('p') || key.code == KeyCode::Left {
                    //    music_player.prev_song();
                    //}
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        while let Some((meta, val)) = info_consumer.try_pop() {
            //println!("{meta} = {val}");
        }
    }
    disable_raw_mode().unwrap();
    stdout().execute(LeaveAlternateScreen).unwrap();
    Ok(())
}
