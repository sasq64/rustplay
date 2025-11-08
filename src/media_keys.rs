use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use zbus::Connection;
use zbus::interface;

#[derive(Clone, Copy, Default)]
pub struct CallCounts {
    pub next_count: usize,
    pub previous_count: usize,
    pub play_pause_count: usize,
    pub is_playing: bool,
}

/// Main MPRIS interface implementation
pub struct MainInterface {
    call_counts: Arc<Mutex<CallCounts>>,
}

impl MainInterface {
    fn new(call_counts: Arc<Mutex<CallCounts>>) -> Self {
        MainInterface { call_counts }
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
    call_counts: Arc<Mutex<CallCounts>>,
}

impl MediaPlayer {
    fn new(call_counts: Arc<Mutex<CallCounts>>) -> Self {
        MediaPlayer { call_counts }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MediaPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        if let Ok(counts) = self.call_counts.lock() {
            if counts.is_playing {
                "Playing".to_string()
            } else {
                "Stopped".to_string()
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
        if let Ok(counts) = self.call_counts.lock() {
            !counts.is_playing
        } else {
            true
        }
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        if let Ok(counts) = self.call_counts.lock() {
            counts.is_playing
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
        println!("[MPRIS] Next pressed");
        if let Ok(mut counts) = self.call_counts.lock() {
            counts.next_count += 1;
        }
        Ok(())
    }

    fn previous(&self) -> zbus::fdo::Result<()> {
        println!("[MPRIS] Previous pressed");
        if let Ok(mut counts) = self.call_counts.lock() {
            counts.previous_count += 1;
        }
        Ok(())
    }

    fn play_pause(&self) -> zbus::fdo::Result<()> {
        println!("[MPRIS] PlayPause pressed");
        if let Ok(mut counts) = self.call_counts.lock() {
            counts.play_pause_count += 1;
            counts.is_playing = !counts.is_playing;
        }
        Ok(())
    }

    fn play(&self) -> zbus::fdo::Result<()> {
        println!("[MPRIS] Play pressed");
        if let Ok(mut counts) = self.call_counts.lock() {
            counts.is_playing = true;
        }
        Ok(())
    }

    fn pause(&self) -> zbus::fdo::Result<()> {
        println!("[MPRIS] Pause pressed");
        if let Ok(mut counts) = self.call_counts.lock() {
            counts.is_playing = false;
        }
        Ok(())
    }

    fn stop(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }
}

/// Start the MPRIS listener in a background thread
/// Returns (shutdown_flag, call_counts, service_name)
pub fn start_with_name(service_name: &str) -> (Arc<AtomicBool>, Arc<Mutex<CallCounts>>, String) {
    let shutdown = Arc::new(AtomicBool::new(false));
    let call_counts = Arc::new(Mutex::new(CallCounts::default()));
    let shutdown_clone = shutdown.clone();
    let counts_clone = call_counts.clone();
    let service_name_owned = service_name.to_string();
    let service_name_for_thread = service_name_owned.clone();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            match setup_mpris(shutdown_clone, counts_clone, &service_name_for_thread).await {
                Ok(_) => {
                    println!("[MPRIS] Listener started successfully");
                }
                Err(e) => {
                    eprintln!("[MPRIS] Error starting listener: {}", e);
                }
            }
        });
    });

    (shutdown, call_counts, service_name_owned)
}

/// Start the MPRIS listener in a background thread with default service name
/// Returns (shutdown_flag, call_counts)
pub fn start() -> (Arc<AtomicBool>, Arc<Mutex<CallCounts>>) {
    let (shutdown, call_counts, _) = start_with_name("org.mpris.MediaPlayer2.oldplay");
    (shutdown, call_counts)
}

async fn setup_mpris(
    shutdown: Arc<AtomicBool>,
    call_counts: Arc<Mutex<CallCounts>>,
    service_name: &str,
) -> Result<(), zbus::Error> {
    let connection = Connection::session().await?;

    // Request the MPRIS service name
    connection.request_name(service_name).await?;

    // Register both the main interface and the player interface at the standard MPRIS path
    let main = MainInterface::new(call_counts.clone());
    let player = MediaPlayer::new(call_counts);

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
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
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
        let (shutdown, call_counts, _) = start_with_name(service_name);

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

        // Wait for the counter to be incremented with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        loop {
            let counts = call_counts.lock().expect("Failed to lock call counts");
            if counts.next_count >= 1 {
                break;
            }
            drop(counts);

            if attempts >= max_attempts {
                panic!("Timeout waiting for Next call to be recorded");
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        }

        // Verify the counter was incremented
        let counts = call_counts.lock().expect("Failed to lock call counts");
        assert_eq!(
            counts.next_count, 1,
            "Expected 1 Next call, got {}",
            counts.next_count
        );

        // Cleanup
        shutdown.store(true, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn test_mpris_previous() {
        let service_name = "org.mpris.MediaPlayer2.oldplay.previous_test";
        let (shutdown, call_counts, _) = start_with_name(service_name);

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

        // Wait for the counter to be incremented with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        loop {
            let counts = call_counts.lock().expect("Failed to lock call counts");
            if counts.previous_count >= 1 {
                break;
            }
            drop(counts);

            if attempts >= max_attempts {
                panic!("Timeout waiting for Previous call to be recorded");
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        }

        let counts = call_counts.lock().expect("Failed to lock call counts");
        assert_eq!(
            counts.previous_count, 1,
            "Expected 1 Previous call, got {}",
            counts.previous_count
        );

        shutdown.store(true, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn test_mpris_play_pause() {
        let service_name = "org.mpris.MediaPlayer2.oldplay.playpause_test";
        let (shutdown, call_counts, _) = start_with_name(service_name);

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

        // Wait for the counter to be incremented with a timeout
        let mut attempts = 0;
        let max_attempts = 30;
        loop {
            let counts = call_counts.lock().expect("Failed to lock call counts");
            if counts.play_pause_count >= 1 {
                break;
            }
            drop(counts);

            if attempts >= max_attempts {
                panic!("Timeout waiting for PlayPause call to be recorded");
            }
            thread::sleep(Duration::from_millis(50));
            attempts += 1;
        }

        let counts = call_counts.lock().expect("Failed to lock call counts");
        assert_eq!(
            counts.play_pause_count, 1,
            "Expected 1 PlayPause call, got {}",
            counts.play_pause_count
        );

        shutdown.store(true, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(200));
    }
}
