use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
pub struct Fifo<T> {
    deque: Arc<Mutex<VecDeque<T>>>,
    cv: Arc<Condvar>,
}

#[allow(dead_code)]
impl<T> Fifo<T> {
    pub fn push_back(&mut self, v: T) {
        loop {
            let mut guard = self.deque.lock().unwrap();
            if guard.len() > 64 * 1024 {
                guard = self.cv.wait(guard).expect("wait failed");
                if guard.len() > 64 * 1024 {
                    guard.push_back(v);
                    return;
                }
            } else {
                guard.push_back(v);
                return;
            }
        }
    }
    pub fn pop_front(&mut self) -> Option<T> {
        let result = self.deque.lock().unwrap().pop_front();
        self.cv.notify_one();
        result
    }

    pub fn new() -> Fifo<T> {
        Fifo {
            deque: Arc::new(Mutex::new(VecDeque::new())),
            cv: Arc::new(Condvar::new()),
        }
    }

    pub fn len(&mut self) -> usize {
        self.deque.lock().unwrap().len()
    }
}

#[test]
fn test_fifo() {
    let mut fifo = Fifo::<u32>::new();

    let mut thread_fifo = fifo.clone();
    std::thread::spawn(move || {
        thread_fifo.push_back(1);
    });
    std::thread::sleep(std::time::Duration::from_millis(100));

    let value = fifo.pop_front().unwrap();
    assert_eq!(1, value);
}
