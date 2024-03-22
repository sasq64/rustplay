use std::io::stdout;

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, widgets::*};

mod music_player;

mod audio_player;
mod fifo;

use music_player::MusicPlayer;
use music_player::PlayerInfo;

#[cfg(target_os = "linux")]
mod linux_player;

#[cfg(target_os = "macos")]
mod mac_player;

#[cfg(target_os = "windows")]
mod win_player;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut music_player = MusicPlayer::create("data");
    music_player.play(&args[1]);
    music_player.set_song(0);
    // let mp = Rc::new(RefCell::new(music_player));

    enable_raw_mode().unwrap();
    stdout().execute(EnterAlternateScreen).unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();

    let ms = std::time::Duration::from_millis(50);

    let mut sub_title: String = "".to_string();
    let mut title: String = "Unknown".to_string();
    let mut song_composer: String = "Unknown".to_string();
    let mut length = 0;
    let mut song = 0;
    let mut songs = 1;
    let mut seconds: f32 = 0.0;

    loop {
        let mut changed = true;
        match music_player.get_info() {
            PlayerInfo::Title(s) => title = s,
            PlayerInfo::Game(s) => title = s,
            PlayerInfo::Seconds(f) => seconds = f,
            PlayerInfo::Subtitle(s) => {
                if s == "" {
                    sub_title = s;
                } else {
                    sub_title = format!(" ({})", s);
                }
            }
            PlayerInfo::Composer(composer) => song_composer = composer,
            PlayerInfo::Length(i) => length = i,
            PlayerInfo::Song(i) => song = i,
            PlayerInfo::Songs(i) => songs = i,
            _ => changed = false,
        }
        if changed {
            terminal.draw(|frame| {
                let secs = seconds as i32;
                let t = format!("TITLE   : {}{}\nCOMPOSER: {}\nLENGTH  : {:02}:{:02} / {:02}:{:02}\nSONG    : {:02}/{:02}",
                                title, sub_title, song_composer, secs / 60, secs % 60, length / 60, length % 60, song + 1, songs);
                frame.render_widget(
                    Paragraph::new(t).block(Block::default().title("Play Music").borders(Borders::ALL)),
                    frame.size(),
                )
            }).unwrap();
        }
        if event::poll(ms).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == event::KeyEventKind::Press {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    if key.code == KeyCode::Char('n') || key.code == KeyCode::Right {
                        music_player.next_song();
                    }
                    if key.code == KeyCode::Char('p') || key.code == KeyCode::Left {
                        music_player.prev_song();
                    }
                }
            }
        }
    }

    disable_raw_mode().unwrap();
    stdout().execute(LeaveAlternateScreen).unwrap();
}
