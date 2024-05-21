use std::collections::HashMap;
use std::io::{self, stdout, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::{error::Error, path::Path, thread};

use cpal::{traits::*, SupportedBufferSize};
use crossterm::cursor::{Hide, Show};
use musix::ChipPlayer;
use new_string_template::template::Template;
use ringbuf::{traits::*, StaticRb};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand, QueueableCommand,
};

struct Player {
    chip_player: ChipPlayer,
    song: i32,
    millis: Arc<AtomicUsize>,
}

impl Player {
    pub fn reset(&mut self) {
        self.millis.store(0, Ordering::SeqCst);
    }

    pub fn next(&mut self) {
        self.song += 1;
        self.chip_player.seek(self.song, 0);
        self.reset();
    }
    pub fn prev(&mut self) {
        self.song -= 1;
        self.chip_player.seek(self.song, 0);
        self.reset();
    }

    pub fn load(&mut self, name: &str) {
        self.chip_player = musix::load_song(name).unwrap();
        self.reset();
    }
}

type Cmd = Box<dyn FnOnce(&mut Player) -> () + Send>;
type Info = (String, String);

fn run_player<P, C>(
    mut info_producer: P,
    mut cmd_consumer: C,
    msec_counter: Arc<AtomicUsize>,
) -> Result<(), Box<dyn Error>>
where
    P: Producer<Item = Info> + Send + 'static,
    C: Consumer<Item = Cmd> + Send + 'static,
{
    musix::init(Path::new("data"))?;

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

    let msec_outside = msec_counter.clone();

    thread::spawn(move || {
        let ring = StaticRb::<f32, 32768>::default();
        let (mut producer, mut consumer) = ring.split();

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumer.pop_slice(data);
                    let ms = data.len() * 1000 / (44100 * 2);
                    msec_counter.fetch_add(ms, Ordering::SeqCst);
                },
                |err| eprintln!("an error occurred on stream: {err}"),
                None,
            )
            .unwrap();
        stream.play().unwrap();
        let mut target: Vec<i16> = vec![0; buffer_size as usize];

        let mut player = Player {
            chip_player: musix::ChipPlayer::new(),
            song: 0,
            millis: msec_outside,
        };

        loop {
            let song = player.song;
            while let Some(f) = cmd_consumer.try_pop() {
                f(&mut player);
            }
            if song != player.song {
                let _ = info_producer.try_push(("song".to_owned(), player.song.to_string()));
            }

            while let Some(meta) = player.chip_player.get_changed_meta() {
                let val = player.chip_player.get_meta_string(&meta).unwrap();
                if meta == "startSong" {
                    player.song = val.parse::<i32>().unwrap();
                }
                let _ = info_producer.try_push((meta, val));
            }
            if producer.vacant_len() > target.len() {
                player.chip_player.get_samples(&mut target);
                producer.push_iter(target.iter().map(|i| (*i as f32) / 32767.0));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    });
    Ok(())
}

fn handle_keys<P>(cmd_producer: &mut P) -> Result<bool, io::Error>
where
    P: Producer<Item = Cmd>,
{
    let ms = std::time::Duration::from_millis(20);
    if event::poll(ms)? {
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press {
                if key.code == KeyCode::Char('q') {
                    return Ok(true);
                }
                if key.code == KeyCode::Char('n') || key.code == KeyCode::Right {
                    let _ = cmd_producer.try_push(Box::new(|p| {
                        p.next();
                    }));
                }
                if key.code == KeyCode::Char('p') || key.code == KeyCode::Left {
                    let _ = cmd_producer.try_push(Box::new(|p| {
                        p.prev();
                    }));
                }
            }
        }
    }
    return Ok(false);
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("No song to play!");
        return Ok(());
    }

    let cmd_buf = StaticRb::<Cmd, 32>::default();
    let (mut cmd_producer, cmd_consumer) = cmd_buf.split();

    let info_buf = StaticRb::<Info, 64>::default();
    let (info_producer, mut info_consumer) = info_buf.split();

    let msec = Arc::new(AtomicUsize::new(0));

    let _ = run_player(info_producer, cmd_consumer, msec.clone());

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?.execute(Hide)?;

    let _ = cmd_producer.try_push(Box::new(move |p| p.load(&args[1])));

    let mut song_meta = HashMap::<String, String>::new();
    let mut meta_changed = true;

    let templ = Template::new("TITLE:    {title}\r\n          {sub_title}\r\nCOMPOSER: {composer}\r\nFORMAT:   {format}\r\n\nTIME: 00:00:00 ({len}) SONG: {song}/{songs}");

    loop {
        let do_quit = handle_keys(&mut cmd_producer)?;
        if do_quit {
            break;
        }

        if meta_changed {
            meta_changed = false;
            let text = templ.render_nofail_string(&song_meta);
            let _ = stdout()
                .queue(Clear(ClearType::All))?
                .queue(cursor::MoveTo(0, 0))?
                .queue(Print(text))?;
        }
        let play_time = msec.load(Ordering::SeqCst);
        let c = (play_time / 10) % 100;
        let m = play_time / 60000;
        let s = (play_time / 1000) % 60;
        let _ = stdout()
            .queue(cursor::MoveTo(6, 5))?
            .queue(Print(format!("{:02}:{:02}:{:02}", m, s, c)))?
            .flush();

        while let Some((meta, val)) = info_consumer.try_pop() {
            if meta == "length" {
                let s = val.parse::<i32>()?;
                song_meta.insert(
                    "len".to_owned(),
                    format!("{:02}:{:02}", s / 60, s % 60).to_owned(),
                );
            }
            song_meta.insert(meta, val);
            meta_changed = true;
        }
    }
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?.execute(Show)?;
    Ok(())
}
