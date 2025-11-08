use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use zbus::Connection;
use zbus::interface;

use crate::log;

/// Media key events that can be listened to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKeyEvent {
    Next,
    Previous,
    PlayPause,
    Play,
    Pause,
    Stop,
    Shutdown,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayState {
    is_playing: bool,
}

/// Main MPRIS interface implementation
pub struct MainInterface {
    event_sender: mpsc::Sender<MediaKeyEvent>,
}

impl MainInterface {
    fn new(event_sender: mpsc::Sender<MediaKeyEvent>) -> Self {
        MainInterface { event_sender }
    }
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl MainInterface {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Oldplay".to_string()
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    fn quit(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    fn raise(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }
}

/// MPRIS Media Player interface implementation
pub struct MediaPlayer {
    play_state: Arc<Mutex<PlayState>>,
    event_sender: mpsc::Sender<MediaKeyEvent>,
}

impl MediaPlayer {
    fn new(play_state: Arc<Mutex<PlayState>>, event_sender: mpsc::Sender<MediaKeyEvent>) -> Self {
        MediaPlayer {
            play_state,
            event_sender,
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MediaPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        if let Ok(ps) = self.play_state.lock() {
            if ps.is_playing {
                "Playing".to_string()
            } else {
                "Paused".to_string()
            }
        } else {
            "Stopped".to_string()
        }
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::Value> {
        std::collections::HashMap::new()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        if let Ok(play_state) = self.play_state.lock() {
            log!("can_play {}", !play_state.is_playing);
            !play_state.is_playing
        } else {
            true
        }
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        if let Ok(play_state) = self.play_state.lock() {
            log!("can_pause {}", play_state.is_playing);
            play_state.is_playing
        } else {
            true
        }
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    fn next(&self) -> zbus::fdo::Result<()> {
        log!("[MPRIS] Next pressed");
        let _ = self.event_sender.send(MediaKeyEvent::Next);
        Ok(())
    }

    fn previous(&self) -> zbus::fdo::Result<()> {
        log!("[MPRIS] Previous pressed");
        let _ = self.event_sender.send(MediaKeyEvent::Previous);
        Ok(())
    }

    fn play_pause(&self) -> zbus::fdo::Result<()> {
        log!("[MPRIS] PlayPause pressed");
        let _ = self.event_sender.send(MediaKeyEvent::PlayPause);
        Ok(())
    }

    fn play(&self) -> zbus::fdo::Result<()> {
        log!("[MPRIS] Play pressed");
        let _ = self.event_sender.send(MediaKeyEvent::Play);
        Ok(())
    }

    fn pause(&self) -> zbus::fdo::Result<()> {
        log!("[MPRIS] Pause pressed");
        let _ = self.event_sender.send(MediaKeyEvent::Pause);
        Ok(())
    }

    fn stop(&self) -> zbus::fdo::Result<()> {
        let _ = self.event_sender.send(MediaKeyEvent::Stop);
        Ok(())
    }
}

/// Start the MPRIS listener in a background thread
/// Returns (shutdown_sender, event_receiver, service_name)
pub fn start_with_name(
    service_name: &str,
) -> (
    mpsc::Sender<MediaKeyEvent>,
    mpsc::Receiver<MediaKeyEvent>,
    String,
) {
    let play_state = Arc::new(Mutex::new(PlayState::default()));
    let (event_sender, event_receiver) = mpsc::channel();
    let (shutdown_sender, shutdown_receiver) = mpsc::channel();
    let state_clone = play_state.clone();
    let service_name_owned = service_name.to_string();
    let service_name_for_thread = service_name_owned.clone();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            match setup_mpris(
                shutdown_receiver,
                state_clone,
                &service_name_for_thread,
                event_sender,
            )
            .await
            {
                Ok(_) => {
                    println!("[MPRIS] Listener started successfully");
                }
                Err(e) => {
                    eprintln!("[MPRIS] Error starting listener: {}", e);
                }
            }
        });
    });

    (shutdown_sender, event_receiver, service_name_owned)
}

/// Start the MPRIS listener in a background thread with default service name
/// Returns (shutdown_sender, event_receiver)
pub fn start() -> (mpsc::Sender<MediaKeyEvent>, mpsc::Receiver<MediaKeyEvent>) {
    let (shutdown_sender, event_receiver, _) = start_with_name("org.mpris.MediaPlayer2.oldplay");
    (shutdown_sender, event_receiver)
}

async fn setup_mpris(
    event_receiver: mpsc::Receiver<MediaKeyEvent>,
    play_state: Arc<Mutex<PlayState>>,
    service_name: &str,
    event_sender: mpsc::Sender<MediaKeyEvent>,
) -> Result<(), zbus::Error> {
    let connection = Connection::session().await?;

    // Request the MPRIS service name
    connection.request_name(service_name).await?;

    // Register both the main interface and the player interface at the standard MPRIS path
    let main = MainInterface::new(event_sender.clone());
    let player = MediaPlayer::new(play_state.clone(), event_sender);

    connection
        .object_server()
        .at("/org/mpris/MediaPlayer2", main)
        .await?;

    connection
        .object_server()
        .at("/org/mpris/MediaPlayer2", player)
        .await?;

    println!("[MPRIS] Registered as {}", service_name);

    // Keep the connection alive until shutdown is signaled
    loop {
        match event_receiver.try_recv() {
            Ok(MediaKeyEvent::Shutdown) => break,
            Ok(MediaKeyEvent::Playing) => {
                log!("PLAY");
                if let Ok(mut ps) = play_state.lock() {
                    ps.is_playing = true
                }
            }
            Ok(MediaKeyEvent::Paused) => {
                log!("PAUSE");
                if let Ok(mut ps) = play_state.lock() {
                    ps.is_playing = false
                }
            }
            Ok(_) => {} // Ignore other events on this receiver
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    connection.release_name(service_name).await?;
    println!("[MPRIS] Listener stopped");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Duration;

    fn verify_service_registered(service_name: &str) -> bool {
        // Use busctl to check if the service is registered
        let output = Command::new("busctl")
            .args(&["--user", "list", "--no-pager"])
            .output()
            .expect("Failed to execute busctl");

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains(service_name)
    }

    #[test]
    fn test_mpris_next() {
        // Start the listener with a unique name for this test
        let service_name = "org.mpris.MediaPlayer2.oldplay.next_test";
        let (event_sender, event_receiver, _) = start_with_name(service_name);

        // Wait for D-Bus registration with verification
        let mut attempts = 0;
        while !verify_service_registered(service_name) && attempts < 20 {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert!(
            verify_service_registered(service_name),
            "MPRIS service not registered on D-Bus"
        );

        // Call Next via busctl
        let output = Command::new("busctl")
            .args([
                "call",
                "--user",
                service_name,
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
                "Next",
            ])
            .output()
            .expect("Failed to execute busctl");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            output.status.success(),
            "busctl call failed with status {}: stderr='{}' stdout='{}'",
            output.status.code().unwrap_or(-1),
            stderr,
            stdout
        );

        // Wait for the event to be received with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        let event_received = loop {
            if let Ok(event) = event_receiver.try_recv() {
                break Some(event);
            }

            if attempts >= max_attempts {
                break None;
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        };

        // Verify the event was received
        assert_eq!(
            event_received,
            Some(MediaKeyEvent::Next),
            "Expected MediaKeyEvent::Next"
        );

        // Cleanup
        let _ = event_sender.send(MediaKeyEvent::Shutdown);
        thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn test_mpris_previous() {
        let service_name = "org.mpris.MediaPlayer2.oldplay.previous_test";
        let (event_sender, event_receiver, _) = start_with_name(service_name);

        let mut attempts = 0;
        while !verify_service_registered(service_name) && attempts < 20 {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert!(
            verify_service_registered(service_name),
            "MPRIS service not registered on D-Bus"
        );

        let output = Command::new("busctl")
            .args([
                "call",
                "--user",
                service_name,
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
                "Previous",
            ])
            .output()
            .expect("Failed to execute busctl");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            output.status.success(),
            "busctl call failed with status {}: stderr='{}' stdout='{}'",
            output.status.code().unwrap_or(-1),
            stderr,
            stdout
        );

        // Wait for the event to be received with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        let event_received = loop {
            if let Ok(event) = event_receiver.try_recv() {
                break Some(event);
            }

            if attempts >= max_attempts {
                break None;
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        };

        // Verify the event was received
        assert_eq!(
            event_received,
            Some(MediaKeyEvent::Previous),
            "Expected MediaKeyEvent::Previous"
        );

        let _ = event_sender.send(MediaKeyEvent::Shutdown);
        thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn test_mpris_play_pause() {
        let service_name = "org.mpris.MediaPlayer2.oldplay.playpause_test";
        let (event_sender, event_receiver, _) = start_with_name(service_name);

        let mut attempts = 0;
        while !verify_service_registered(service_name) && attempts < 20 {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert!(
            verify_service_registered(service_name),
            "MPRIS service not registered on D-Bus"
        );

        let output = Command::new("busctl")
            .args([
                "call",
                "--user",
                service_name,
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
                "PlayPause",
            ])
            .output()
            .expect("Failed to execute busctl");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(
            output.status.success(),
            "busctl call failed with status {}: stderr='{}' stdout='{}'",
            output.status.code().unwrap_or(-1),
            stderr,
            stdout
        );

        // Wait for the event to be received with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        let event_received = loop {
            if let Ok(event) = event_receiver.try_recv() {
                break Some(event);
            }

            if attempts >= max_attempts {
                break None;
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        };

        // Verify the event was received
        assert_eq!(
            event_received,
            Some(MediaKeyEvent::PlayPause),
            "Expected MediaKeyEvent::PlayPause"
        );

        let _ = event_sender.send(MediaKeyEvent::Shutdown);
        thread::sleep(Duration::from_millis(200));
    }
}
