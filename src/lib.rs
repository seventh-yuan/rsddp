use websocket::client::ClientBuilder;
use websocket as ws;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use json::{object, JsonValue};
use std::convert::From;
use std::vec::Vec;
use std::collections::HashMap;
use std::sync::{Arc,Mutex,Condvar};
use std::thread::{self, spawn, sleep};
use std::time::{Duration, Instant};


#[derive(Debug)]
pub enum DDPMessage {
    Connect(JsonValue),
    Connected(JsonValue),
    Failed(JsonValue),
    Method(JsonValue),
    Result(JsonValue),
    Updated(JsonValue),
    Sub(JsonValue),
    UnSub(JsonValue),
    NoSub(JsonValue),
    Added(JsonValue),
    Changed(JsonValue),
    Removed(JsonValue),
    Ready(JsonValue),
    AddedBefore(JsonValue),
    MovedBefore(JsonValue),
    Ping(JsonValue),
    Pong(JsonValue),
    Invalid(String),
}

#[derive(Debug)]
pub enum DDPError {
    Timeout(String),
    InvalidMessage(String),
    SendFailed(String),
    ConnFailed(String),
    NotMatching(String),
    Other(String),
}

impl From<&str> for DDPMessage {
    fn from(msg: &str) -> Self {
        let msg = json::parse(msg).unwrap();
        if msg["msg"] == "connect" {
            return DDPMessage::Connect(msg);
        } else if msg["msg"] == "connected" {
            return DDPMessage::Connected(msg);
        } else if msg["msg"] == "failed" {
            return DDPMessage::Failed(msg);
        } else if msg["msg"] == "ping" {
            return DDPMessage::Ping(msg);
        } else if msg["msg"] == "pong" {
            return DDPMessage::Pong(msg);
        } else if msg["msg"] == "sub" {
            return DDPMessage::Sub(msg);
        } else if msg["msg"] == "unsub" {
            return DDPMessage::UnSub(msg);
        } else if msg["msg"] == "nosub" {
            return DDPMessage::NoSub(msg);
        } else if msg["msg"] == "added" {
            return DDPMessage::Added(msg);
        } else if msg["msg"] == "changed" {
            return DDPMessage::Changed(msg);
        } else if msg["msg"] == "removed" {
            return DDPMessage::Removed(msg);
        } else if msg["msg"] == "ready" {
            return DDPMessage::Ready(msg);
        } else if msg["msg"] == "addedBefore" {
            return DDPMessage::AddedBefore(msg);
        } else if msg["msg"] == "movedBefore" {
            return DDPMessage::MovedBefore(msg);
        } else if msg["msg"] == "method" {
            return DDPMessage::Method(msg);
        } else if msg["msg"] == "result" {
            return DDPMessage::Result(msg);
        } else if msg["msg"] == "updated" {
            return DDPMessage::Updated(msg);
        } else {
            return DDPMessage::Invalid(msg.to_string());
        }
    }
}

impl DDPMessage {
    fn connect(session: &str, version: &str, support: JsonValue) -> JsonValue {
        object!{
            "msg": "connect",
            "session": session,
            "version": version,
            "support": support
        }
    }

    fn connected(session: &str) -> JsonValue {
        object!{
            "msg": "connected",
            "session": session
        }
    }

    fn failed(version: &str) -> JsonValue {
        object!{
            "msg": "failed",
            "version": version
        }
    }

    fn sub(id: &str, name: &str, params: JsonValue) -> JsonValue {
        object!{
            "msg": "sub",
            "id": id,
            "name": name,
            "params": params,
        }
    }

    fn unsub(id: &str) -> JsonValue {
        object!{
            "msg": "unsub",
            "id": id
        }
    }

    fn nosub(id: &str, error: JsonValue) -> JsonValue {
        object!{
            "msg": "nosub",
            "id": id,
            "error": error
        }
    }

    fn added(collection: &str, id: &str, fields: JsonValue) -> JsonValue {
        object!{
            "msg": "added",
            "collection": collection,
            "id": id,
            "fields": fields
        }
    }

    fn changed(collection: &str, id: &str, fields: JsonValue) -> JsonValue {
        object!{
            "msg": "changed",
            "collection": collection,
            "id": id,
            "fields": fields
        }
    }

    fn removed(collection: &str, id: &str) -> JsonValue {
        object!{
            "msg": "removed",
            "collection": collection,
            "id": id,
        }
    }

    fn ready(subs: JsonValue) -> JsonValue {
        object!{
            "msg": "ready",
            "subs": subs
        }
    }

    fn method(method: &str, params: JsonValue, id: &str, randomSeed: JsonValue) -> JsonValue {
        object!{
            "msg": "method",
            "method": method,
            "params": params,
            "id": id,
            "randomSeed": randomSeed
        }
    }

    fn result(id: &str, error: JsonValue, result: JsonValue) -> JsonValue {
        object!{
            "msg": "result",
            "id": id,
            "error": error,
            "result": result
        }
    }

    fn updated(methods: JsonValue) -> JsonValue {
        object!{
            "msg": "updated",
            "methods": methods
        }
    }
}

pub struct Event {
    msgs: Mutex<HashMap<String, DDPMessage>>,
    cond: Condvar,
}

impl Event {
    pub fn new() -> Self {
        Self {
            msgs: Mutex::new(HashMap::new()),
            cond: Condvar::new(),
        }
    }

    pub fn wait(&self, id: &str) -> Result<DDPMessage, DDPError> {
        let mut msgs = self.msgs.lock().unwrap();
        while msgs.len() <= 0 {
            msgs = self.cond.wait(msgs).unwrap();
        }
        let msg = match msgs.remove(id) {
            Some(msg) => msg,
            None => {
                return Err(DDPError::Other("invalid event found".to_string()));
            }
        };
        return Ok(msg);
    }

    pub fn wait_timeout(&self, id: &str, timeout: Duration) ->Result<DDPMessage, DDPError> {
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

    pub fn notify(&self, id: String, msg: DDPMessage) {
        let mut msgs = self.msgs.lock().unwrap();
        msgs.insert(id.clone(), msg);
        self.cond.notify_all();
    }
}

pub struct DDPClient {
    ws_tx: Sender<ws::OwnedMessage>,
    threads: Vec<JoinHandle<()>>,
    conn_event: Arc<Event>,
    pong_event: Arc<Event>,
    sub_event: Arc<Event>,
    result_event: Arc<Event>,
    updated_event: Arc<Event>,
    session: String,
    method_id: u32,
}

impl Drop for DDPClient {
    fn drop(&mut self) {
        self.send_message(ws::OwnedMessage::Close(None));
        while let Some(th) = self.threads.pop() {
            th.join().unwrap();
        }
    }
}

impl DDPClient {
    pub fn connect(endpoint: &str, timeout: Duration) -> Result<Self, DDPError> {
        let client = ClientBuilder::new(endpoint).unwrap().connect_insecure().unwrap();
        let (mut receiver, mut sender) = client.split().unwrap();

        let (ws_tx, ws_rx) = mpsc::channel();

        let ws_tx_clone = ws_tx.clone();
        let mut threads = Vec::new();
        let conn_event = Arc::new(Event::new());
        let conn_event_clone = conn_event.clone();
        let pong_event = Arc::new(Event::new());
        let pong_event_clone = pong_event.clone();
        let sub_event = Arc::new(Event::new());
        let sub_event_clone = sub_event.clone();
        let result_event = Arc::new(Event::new());
        let result_event_clone = result_event.clone();
        let updated_event = Arc::new(Event::new());
        let updated_event_clone = updated_event.clone();
        let send_th = thread::spawn(move || {
            loop {
                let message = match ws_rx.recv() {
                    Ok(m) => m,
                    Err(e) => {
                        println!("send loop: {:?}", e);
                        return;
                    }
                };
                match message {
                    ws::OwnedMessage::Close(_) => {
                        let _ = sender.send_message(&message);
                        return;
                    }
                    _ => {
                        sender.send_message(&message).unwrap();
                    }
                }

            }
        });
        threads.push(send_th);
        let recv_th = thread::spawn(move || {
            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Receive loop: {:?}", e);
                        let _ = ws_tx.send(ws::OwnedMessage::Close(None));
                        return;
                    }
                };
                let message = match message {
                    ws::OwnedMessage::Text(m) => DDPMessage::from(&m[..]),
                    ws::OwnedMessage::Close(_) => {
                        let _ = ws_tx.send(ws::OwnedMessage::Close(None));
                        return;
                    }
                    ws::OwnedMessage::Ping(data) => {
                        match ws_tx.send(ws::OwnedMessage::Pong(data)) {
                            Ok(()) => (),
                            Err(e) => {
                                println!("Recevie loop: {:?}", e);
                                return;
                            }
                        };
                        return;
                    }
                    _ => {
                        println!("invalid message: {:?}", message);
                        return;
                    }
                };
                match message {
                    DDPMessage::Connected(_) => {
                        conn_event.notify("".to_string(), message);
                    }
                    DDPMessage::Failed(_) => {
                        conn_event.notify("".to_string(), message);
                    }
                    DDPMessage::Result(m) => {
                        result_event.notify(m["id"].to_string(), DDPMessage::Result(m));
                    }
                    DDPMessage::Updated(m) => {

                    }
                    DDPMessage::NoSub(m) => {
                        sub_event.notify(m["id"].to_string(), DDPMessage::NoSub(m));
                    }
                    DDPMessage::Added(m) => {

                    }
                    DDPMessage::Changed(m) => {

                    }
                    DDPMessage::Removed(m) => {

                    }
                    DDPMessage::Ready(m) => {

                    }
                    DDPMessage::AddedBefore(m) => {

                    }
                    DDPMessage::MovedBefore(m) => {

                    }
                    DDPMessage::Pong(m) => {
                        pong_event.notify(m["id"].to_string(), DDPMessage::Pong(m));
                    }
                    _ => {
                        println!("invalid message: {:?}", message);
                        return;
                    }
                }
            }

        });
        threads.push(recv_th);
        let mut client = Self {
            ws_tx: ws_tx_clone,
            threads: threads,
            conn_event: conn_event_clone,
            pong_event: pong_event_clone,
            sub_event: sub_event_clone,
            result_event: result_event_clone,
            updated_event: updated_event_clone,
            session: "".to_string(),
            method_id: 0,
        };

        let ddp_versions = vec!["1", "pre2", "pre1"]; 
        let mut v_index = 0;
        let mut connect_message = DDPMessage::connect("", ddp_versions[v_index], JsonValue::from(ddp_versions.clone()));
        loop {
            v_index += 1;
            if v_index >= 3 {
                return Err(DDPError::NotMatching("no matching version".to_string()));
            }
            client.ws_tx.send(ws::OwnedMessage::Text(connect_message.dump())).unwrap();
            let message = client.conn_event.wait_timeout("", timeout)?;
            match message {
                DDPMessage::Connected(msg) => {
                    client.session = msg["session"].dump();
                    break;
                }
                DDPMessage::Failed(msg) => {
                    connect_message["version"] = JsonValue::from(ddp_versions[v_index]);
                }
                _ => {
                    panic!("invalid message");
                }
            }
        }
        return Ok(client);
    }

    fn send_message(&self, msg: ws::OwnedMessage) -> Result<(), DDPError> {
        match self.ws_tx.send(msg) {
            Ok(()) => Ok(()),
            Err(_) => Err(DDPError::SendFailed("send message failed".to_string()))
        }
    }

    pub fn call(&mut self, method: String, params: JsonValue, timeout: Duration) -> Result<JsonValue, DDPError> {
        let method_id = self.method_id.to_string();
        self.method_id += 1;
        let message = DDPMessage::method(&method[..], params, &method_id[..], "".into());
        self.send_message(ws::OwnedMessage::Text(JsonValue::dump(&message)))?;
        let msg = self.result_event.wait_timeout(&method_id[..], timeout)?;
        let msg = match msg {
            DDPMessage::Result(m) => m,
            _ => {
                return Err(DDPError::InvalidMessage("invalid ddp message".to_string()));
            }
        };
        return Ok(msg);
    }
}
