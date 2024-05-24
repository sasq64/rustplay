#![allow(dead_code)]

use cpal::traits::*;
use crossterm::cursor::{Hide, Show};
use crossterm::style::Color;
use crossterm::event::KeyModifiers;
use crossterm::style::SetForegroundColor;
use musix::ChipPlayer;
use ringbuf::{traits::*, StaticRb};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as XWrite;
use std::io::Write as YWrite;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::{error::Error, path::Path, thread};
use strfmt::{strfmt, DisplayStr};

use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{samples_fft_to_spectrum, FrequencyLimit};

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

mod player;

struct Player {
    chip_player: ChipPlayer,
    song: i32,
    millis: Arc<AtomicUsize>,
    song_queue: VecDeque<PathBuf>,
    playing: bool,
    quitting: bool,
}

impl Player {
    pub fn reset(&mut self) {
        self.millis.store(0, Ordering::SeqCst);
    }

    pub fn next(&mut self) {
        if let Some(song_file) = self.song_queue.pop_front() {
            self.load(&song_file);
        }
    }

    pub fn add_song(&mut self, path: &Path) {
        self.song_queue.push_back(path.to_owned());
    }

    pub fn next_song(&mut self) {
        self.chip_player.seek(self.song + 1, 0);
        self.reset();
    }
    pub fn prev_song(&mut self) {
        self.chip_player.seek(self.song - 1, 0);
        self.reset();
    }

    pub fn load(&mut self, name: &Path) {
        self.chip_player = ChipPlayer::new();
        self.chip_player = musix::load_song(name).unwrap();
        self.reset();
        self.playing = true
    }

    pub fn quit(&mut self) {
        self.quitting = true;
    }
}

enum Value {
    Text(String),
    Number(i32),
    Data(Vec<u8>)
}

impl DisplayStr for Value {
    fn display_str(&self, f: &mut strfmt::Formatter) -> strfmt::Result<()> {
        match self {
            Value::Text(s) => f.write_str(s.as_str()).unwrap(),
            Value::Number(n) => write!(f, "{:02}", n).unwrap(),
            Value::Data(_) => ()
        }
        Ok(())
    }
}

type Cmd = Box<dyn FnOnce(&mut Player) + Send>;
type Info = (String, Value);

struct RustPlay<CP, IC> {
    cmd_producer: CP,
    info_consumer: IC,
    templ: &'static str,
    meta_changed: bool,
    msec: Arc<AtomicUsize>,
    song_meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    length: i32,
    data: Vec<f32>,
    player_thread: Option<JoinHandle<()>>,
}

impl<CP, IC> RustPlay<CP, IC>
where
    CP: Producer<Item = Cmd>,
    IC: Consumer<Item = Info>,
{
    fn draw_screen(&mut self) -> io::Result<()> {
        if self.meta_changed {
            self.meta_changed = false;
            //let text = self.templ.render_nofail_string(&self.song_meta);
            let text = strfmt(self.templ, &self.song_meta).unwrap();
            let _ = stdout()
                .queue(Clear(ClearType::All))?
                .queue(cursor::MoveTo(0, 0))?
                .queue(Print(text))?;
        }
        let play_time = self.msec.load(Ordering::SeqCst);
        let c = (play_time / 10) % 100;
        let m = play_time / 60000;
        let s = (play_time / 1000) % 60;
        if let Some(Value::Data(data)) = self.song_meta.get("fft") {
            if self.data.len() != data.len() {
                self.data.resize(data.len(), 0.0);
            }
            for i in 0..data.len() {
                let d = data[i] as f32;
                //if (d > 100.0) {
                //    println!("[{}]", d);
                //}
                if self.data[i] < d {
                    self.data[i] = d; 
                } else {
                    self.data[i] = self.data[i] * 0.75 + d * 0.25;
                }
            }
            let w = data.len();
            let mut area : Vec<char> = vec!(' '; w * 10);
            print_bars(&self.data, &mut area, w, 10);
            for i in 0..10 {
                let _ = stdout().queue(cursor::MoveTo(3, 8 + i));
                let u = (i as usize) * w;
                let s2: String = area[u..(u+w)].iter().collect();
                let _ = stdout().queue(SetForegroundColor(Color::Rgb { r: 0, g: 0x80, b: 0 }))?.
                    queue(Print(s2));
            }
        }
        stdout()
            .queue(cursor::MoveTo(6, 5))?
            .queue(Print(format!("{:02}:{:02}:{:02}", m, s, c)))?
            .flush()?;



        Ok(())
    }

    fn handle_keys(&mut self) -> Result<bool, io::Error> {
        let ms = std::time::Duration::from_millis(20);
        if event::poll(ms)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    // println!("CODE {:?} {:?}", key.code, key.modifiers);
                    if key.code == KeyCode::Char('q') || (key.code == KeyCode::Char('c') && ctrl) {
                        return Ok(true);
                    }
                    if key.code == KeyCode::Char('n') {
                        let _ = self.cmd_producer.try_push(Box::new(|p| {
                            p.next();
            }));
                    }
                    if key.code == KeyCode::Right {
                        let _ = self.cmd_producer.try_push(Box::new(|p| {
                            p.next_song();
                        }));
                    }
                    if key.code == KeyCode::Left {
                        let _ = self.cmd_producer.try_push(Box::new(|p| {
                            p.prev_song();
                        }));
                    }
                }
            }
        }
        Ok(false)
    }

    fn update_meta(&mut self) {
        while let Some((meta, val)) = self.info_consumer.try_pop() {
            match val {
                Value::Number(n) => match meta.as_str() {
                    "length" => {
                        self.song_meta.insert(
                            "len".to_owned(),
                            Value::Text(format!("{:02}:{:02}", n / 60, n % 60).to_owned()),
                        );
                    }
                    "song" => {
                        self.song = n;
                        self.song_meta
                            .insert("isong".to_owned(), Value::Number(n + 1));
                    }
                    "songs" => self.songs = n,
                    &_ => {}
                },
                Value::Text(_) => {},
                Value::Data(_) => {},
            }
            self.song_meta.insert(meta, val);
            self.meta_changed = true;
        }
    }
    fn add_song(&mut self, song: &Path) {
        let pb = song.to_owned();
        let _ = self.cmd_producer.try_push(Box::new(move |p| {
            p.add_song(&pb);
        }));
    }

    fn quit(&mut self) -> Result<(), Box<dyn Error>>  {
        let _ = self.cmd_producer.try_push(Box::new(move |p| {
            p.quit();
        }));
        if let Err(err) = self.player_thread.take().unwrap().join() {
            eprintln!("THREAD ERROR: {:?}", err);
            //eprintln!(msg);
        }
        Ok(())
    }
}

impl RustPlay<(), ()> {
    fn run_player<P, C>(
        mut info_producer: P,
        mut cmd_consumer: C,
        msec: Arc<AtomicUsize>,
    ) -> JoinHandle<()>
    where
        P: Producer<Item = Info> + Send + 'static,
        C: Consumer<Item = Cmd> + Send + 'static,
    {
        musix::init(Path::new("data")).unwrap();

        let device = cpal::default_host().default_output_device().unwrap();
        let mut config = device.default_output_config().unwrap();
        let configs = device.supported_output_configs().unwrap();
        let buffer_size = 4096;
        for conf in configs {
            if let Some(conf2) = conf.try_with_sample_rate(cpal::SampleRate(44100)) {
                config = conf2;
                break;
            }
        }

        let msec_outside = msec.clone();

        thread::spawn(move || {
            let ring = StaticRb::<f32, 8192>::default();
            let (mut producer, mut consumer) = ring.split();

            let stream = device
                .build_output_stream(
                    &config.into(),
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        consumer.pop_slice(data);
                        let ms = data.len() * 1000 / (44100 * 2);
                        msec.fetch_add(ms, Ordering::SeqCst);
                    },
                    |err| eprintln!("An error occurred on stream: {err}"),
                    None,
                )
                .unwrap();
            stream.play().unwrap();

            let mut target: Vec<i16> = vec![0; buffer_size];

            let mut player = Player {
                chip_player: musix::ChipPlayer::new(),
                song: 0,
                millis: msec_outside,
                song_queue: VecDeque::new(),
                playing: false,
                quitting: false
            };

            let mut temp: Vec<f32> = vec![0.0; buffer_size];
            let mut ss : f32 = 0.0;
            let mut xx = 0.2;
            loop {
                if player.quitting {
                    break;
                }
                while let Some(f) = cmd_consumer.try_pop() {
                    f(&mut player);
                }

                if !player.playing {
                    player.next();
                }

                while let Some(meta) = player.chip_player.get_changed_meta() {
                    let val = player.chip_player.get_meta_string(&meta).unwrap();
                    let v: Value = match meta.as_str() {
                        "song" | "startSong" => {
                            player.song = val.parse::<i32>().unwrap();
                            Value::Number(player.song)
                        }
                        "songs" | "length" => {
                            let l = val.parse::<i32>().unwrap();
                            Value::Number(l)
                        }
                        &_ => Value::Text(val),
                    };
                    let _ = info_producer.try_push((meta, v));
                }
                if producer.vacant_len() > target.len() {
                    player.chip_player.get_samples(&mut target);
                    for i in 0..target.len() {
                        temp[i] = (target[i] as f32) / 32767.0;
                        //temp[i] = (ss.sin() + 1.0) / 2.0;
                        ss += xx;
                        if ss > std::f32::consts::PI*2.0 {
                            ss -= std::f32::consts::PI*2.0
                        }
                    }
                    xx -= 0.0005;
                    producer.push_slice(&temp);
                    let hann_window = hann_window(&temp);
                    // calc spectrum
                    let spectrum = samples_fft_to_spectrum(
                        // (windowed) samples
                        &hann_window,
                        //&temp,
                        // sampling rate
                        44100,
                        // optional frequency limit: e.g. only interested in frequencies 50 <= f <= 150?
                        FrequencyLimit::Range(15.0, 1500.0),
                        // optional scale
                        Some(&divide_by_N_sqrt),
                    )
                    .unwrap();
                    //println!("{}", spectrum.data().len());
                    //assert!(spectrum.data().len() == buffer_size/2 -1);


                    //producer.push_iter(target.iter().map(|i| (*i as f32) / 32767.0));
                    let mut data = Vec::new();
                    for chunk in spectrum.data()[1..].chunks(1) {
                        let c: f32 = chunk.iter().map(|(_, j)| j.val()).sum();
                        data.push((c * 80.0) as u8);
                    }
                    let _ = info_producer.try_push(("fft".to_owned(), Value::Data(data)));
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        })
    }

    fn new() -> RustPlay<impl Producer<Item = Cmd>, impl Consumer<Item = Info>> {
        // Send commands to player
        let cmd_buf = StaticRb::<Cmd, 32>::default();
        let (cmd_producer, cmd_consumer) = cmd_buf.split();

        // Receive info from player
        let info_buf = StaticRb::<Info, 64>::default();
        let (info_producer, info_consumer) = info_buf.split();
        let msec = Arc::new(AtomicUsize::new(0));

        let mut song_meta = HashMap::<String, Value>::from([
            ("title".to_owned(), Value::Text("<?>".to_owned())),
            ("song".to_owned(), Value::Number(0)),
            ("isong".to_owned(), Value::Number(1)),
            ("songs".to_owned(), Value::Number(1)),
            ("format".to_owned(), Value::Text("<?>".to_owned())),
            ("composer".to_owned(), Value::Text("<?>".to_owned())),
            ("sub_title".to_owned(), Value::Text("<?>".to_owned())),
        ]);
        song_meta.insert("title".to_owned(), Value::Text("No title".to_owned()));
        RustPlay {
            cmd_producer,
            info_consumer,
            templ: "TITLE:    {title}\r\n          {sub_title}\r\nCOMPOSER: {composer}\r\nFORMAT:   {format}\r\n\nTIME: 00:00:00 ({len}) SONG: {isong}/{songs}",
            meta_changed : false,
            msec: msec.clone(),
            song_meta,
            song : 0,
            songs : 1,
            length : 0,
            data : Vec::new(),
            player_thread : Some(RustPlay::run_player(info_producer, cmd_consumer, msec)) 
        }
    }
}

fn print_bars(bars: &[f32], target: &mut [char], w: usize, h: usize) {
    let c  = ['▁', '▂', '▃','▄','▅','▆','▇','█'];
    for x in 0..bars.len() {
        let n = bars[x];
        for y in 0..h {
            let fy = ((h-y) as f32) * 8.0;
            let d = fy - n;
            target[x + y * w] = if d < 0.0 { '█' } else if d >= 8.0 { '-' } else { c[7-(d as usize)] };
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();

    let mut rust_play = RustPlay::new();

    if args.len() < 2 {
        rust_play.add_song(Path::new("music.s3m"));
        //println!("No song to play!");
        //return Ok(());
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?.execute(Hide)?;

    for song in &args[1..] {
        rust_play.add_song(Path::new(&song));
    }
    loop {
        let do_quit = rust_play.handle_keys()?;
        if do_quit {
            break;
        }
        rust_play.update_meta();
        rust_play.draw_screen()?;
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?.execute(Show)?;

    rust_play.quit()?;

    Ok(())
}
