use std::collections::HashMap;
use std::sync::{Mutex,Condvar};
use std::time::{Duration, Instant};
use crate::error::DDPError;


pub struct Event<T> {
    msgs: Mutex<HashMap<String, T>>,
    cond: Condvar,
}

impl<T> Event<T> {
    pub fn new() -> Self {
        Self {
            msgs: Mutex::new(HashMap::new()),
            cond: Condvar::new(),
        }
    }

    pub fn wait(&self, id: &str, timeout: Duration) ->Result<T, DDPError> {
        let mut msgs = self.msgs.lock().unwrap();
        let ts = Instant::now();
        while msgs.len() <= 0 {
            let duration = Instant::now() - ts;
            if duration >= timeout {
                return Err(DDPError::Timeout("wait event timeout".to_string()));
            }
            let duration = timeout - duration;
            msgs = self.cond.wait_timeout(msgs, duration).unwrap().0;
        }
        let msg = match msgs.remove(id) {
            Some(msg) => msg,
            None => {
                return Err(DDPError::Other("no event found".to_string()));
            }
        };
        return Ok(msg);
    }

    pub fn notify(&self, id: String, msg: T) {
        let mut msgs = self.msgs.lock().unwrap();
        msgs.insert(id.clone(), msg);
        self.cond.notify_all();
    }
}
