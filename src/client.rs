use websocket::client::ClientBuilder;
use websocket as ws;
use std::sync::mpsc;
use websocket::stream::sync::TcpStream;
use std::thread::JoinHandle;
use std::vec::Vec;
use std::sync::{Arc, Mutex, Condvar};
use std::time::{Duration};
use std::thread::{self, spawn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, VecDeque};
use serde_json::Value;
use crate::event::Event;
use crate::message::{ServerMessage, ClientMessage};
use crate::error::DDPError;


struct SubScription {
    id: String,
    on_added: Option<fn(String, String, Option<Value>)>,
    on_changed: Option<fn(String, String, Option<Value>, Option<Value>)>,
    on_removed: Option<fn(String, String)>,
    queue: Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>,
    cond: Arc<Condvar>,
    th: JoinHandle<()>,
}

impl SubScription {
    fn add(&mut self, collection: String, id: String, fields: Option<Value>) {
        let mut queue = self.queue.lock().unwrap();
        let on_added = self.on_added.clone();
        queue.push_back(Box::new(move || {
            if let Some(on_added) = on_added {
                on_added(collection, id, fields);
            }
        }) );
        self.cond.notify_all();
    }

    fn handle_process(queue: Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>, cond: Arc<Condvar>) -> JoinHandle<()> {
        spawn(move || {
            loop {
                let mut msgs = queue.lock().unwrap();
                while msgs.len() <= 0 {
                    msgs = cond.wait(msgs).unwrap();
                }
                if let Some(handle) = msgs.pop_back() {
                    handle();
                }
            }
        })
    }
}

pub struct DDPClient {
    ws_tx: mpsc::Sender<ws::OwnedMessage>,
    threads: Vec<JoinHandle<()>>,
    conn_event: Arc<Event<ServerMessage>>,
    pong_event: Arc<Event<ServerMessage>>,
    sub_event: Arc<Event<ServerMessage>>,
    result_event: Arc<Event<ServerMessage>>,
    updated_event: Arc<Event<ServerMessage>>,
    session: String,
    method_id: u32,
    sub_id: u32,
    alive: Arc<AtomicBool>,
    subs: Arc<Mutex<HashMap<String, SubScription>>>,
}

impl Drop for DDPClient {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.send_ws_message(ws::OwnedMessage::Close(None));
        while let Some(th) = self.threads.pop() {
            th.join().unwrap();
        }
    }
}

impl DDPClient {
    pub fn connect(endpoint: &str, timeout: Duration) -> Result<Self, DDPError> {
        let client = ClientBuilder::new(endpoint).unwrap().connect_insecure().unwrap();
        let (receiver, sender) = client.split().unwrap();

        let (ws_tx, ws_rx) = mpsc::channel();

        let alive_ws_tx = ws_tx.clone();
        let mut threads = Vec::new();
        let conn_event = Arc::new(Event::<ServerMessage>::new());
        let pong_event = Arc::new(Event::<ServerMessage>::new());
        let pong_event_alive = pong_event.clone();
        let sub_event = Arc::new(Event::<ServerMessage>::new());
        let result_event = Arc::new(Event::<ServerMessage>::new());
        let updated_event = Arc::new(Event::<ServerMessage>::new());
        let updated_event_clone = updated_event.clone();
        let subs = Arc::new(Mutex::new(HashMap::new()));
        let send_th = DDPClient::ws_send_loop(ws_rx, sender);
        threads.push(send_th);
        let recv_th = DDPClient::ws_recv_loop(receiver,
                                              ws_tx.clone(),
                                              conn_event.clone(),
                                              result_event.clone(),
                                              pong_event.clone(),
                                              sub_event.clone(),
                                              subs.clone());
        threads.push(recv_th);
        let is_alive = Arc::new(AtomicBool::new(true));
        let mut client = Self {
            ws_tx: ws_tx,
            threads: threads,
            conn_event: conn_event,
            pong_event: pong_event,
            sub_event: sub_event,
            result_event: result_event,
            updated_event: updated_event_clone,
            session: "".to_string(),
            method_id: 0,
            sub_id: 0,
            alive: is_alive.clone(),
            subs: subs,
        };

        let ddp_versions: Vec<String> = vec!["1".to_string(), "pre2".to_string(), "pre1".to_string()]; 
        let mut v_index = 0;
        let mut connect_message = ClientMessage::Connect {
            session: "".to_string(),
            version: ddp_versions[v_index].clone(),
            support: ddp_versions.clone(),
        };

        loop {
            v_index += 1;
            if v_index >= 3 {
                return Err(DDPError::NotMatching("no matching version".to_string()));
            }
            client.ws_tx.send(ws::OwnedMessage::Text(serde_json::to_string(&connect_message).unwrap())).unwrap();
            let message = client.conn_event.wait("", timeout)?;
            match message {
                ServerMessage::Connected {session} => {
                    client.session = session;
                    break;
                }
                ServerMessage::Failed {version} => {
                    if let Some(_index) = ddp_versions.iter().position(|ver| *ver == version) {
                        connect_message = ClientMessage::Connect {
                            session: "".to_string(),
                            version: version,
                            support: ddp_versions.clone(),
                        };
                    } else {
                        return Err(DDPError::NotSupport("no match version found".to_string()));
                    }

                }
                _ => {
                    panic!("invalid message");
                }
            }
        }

        let alive_th = DDPClient::alive_loop(is_alive, alive_ws_tx, pong_event_alive);
        client.threads.push(alive_th);
        return Ok(client);
    }

    fn send_ws_message(&self, msg: ws::OwnedMessage) -> Result<(), DDPError> {
        match self.ws_tx.send(msg) {
            Ok(()) => Ok(()),
            Err(_) => Err(DDPError::SendFailed("send message failed".to_string()))
        }
    }

    fn send_message(&self, msg: String) -> Result<(), DDPError> {
        return self.send_ws_message(ws::OwnedMessage::Text(msg));
    }

    fn ws_send_loop(ws_rx_chan: mpsc::Receiver<ws::OwnedMessage>, mut ws_sender: ws::sender::Writer<TcpStream>) -> JoinHandle<()> {
        thread::spawn(move || {
            loop {
                let message = match ws_rx_chan.recv() {
                    Ok(m) => m,
                    Err(e) => {
                        println!("send loop: {:?}", e);
                        return;
                    }
                };
                match message {
                    ws::OwnedMessage::Close(_) => {
                        let _ = ws_sender.send_message(&message);
                        return;
                    }
                    _ => {
                        ws_sender.send_message(&message).unwrap();
                    }
                }

            }
        })
    }

    fn ws_recv_loop(mut ws_recver: ws::receiver::Reader<TcpStream>,
                    ws_tx_chan: mpsc::Sender<ws::OwnedMessage>,
                    conn_event: Arc<Event<ServerMessage>>,
                    result_event: Arc<Event<ServerMessage>>,
                    pong_event: Arc<Event<ServerMessage>>,
                    sub_event: Arc<Event<ServerMessage>>,
                    subs: Arc<Mutex<HashMap<String, SubScription>>>) -> JoinHandle<()> {
        thread::spawn(move || {
            for message in ws_recver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Receive loop: {:?}", e);
                        let _ = ws_tx_chan.send(ws::OwnedMessage::Close(None));
                        return;
                    }
                };
                let message: ServerMessage = match message {
                    ws::OwnedMessage::Text(m) => serde_json::from_str(&m[..]).unwrap(),
                    ws::OwnedMessage::Close(_) => {
                        let _ = ws_tx_chan.send(ws::OwnedMessage::Close(None));
                        return;
                    }
                    ws::OwnedMessage::Ping(data) => {
                        match ws_tx_chan.send(ws::OwnedMessage::Pong(data)) {
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
                    ServerMessage::Connected {session} => {
                        conn_event.notify("".to_string(), ServerMessage::Connected {session: session});
                    }
                    ServerMessage::Failed {version} => {
                        conn_event.notify("".to_string(), ServerMessage::Failed {version: version});
                    }
                    ServerMessage::Result {id, error, result} => {
                        result_event.notify(id.to_string(), ServerMessage::Result {id: id, error: error, result});
                    }
                    ServerMessage::Pong {id} => {
                        let id = id.unwrap();
                        pong_event.notify(id.to_string(), ServerMessage::Pong {id: Some(id)});

                    }
                    ServerMessage::NoSub {id, error} => {
                        sub_event.notify(id.to_string(), ServerMessage::NoSub {id, error});
                    }
                    ServerMessage::Added {collection, id, fields} => {
                        let sub = &subs.lock().unwrap()[&collection];
                        let on_added = sub.on_added.clone();
                        if let Some(on_added) = on_added {
                            on_added(collection, id, fields);
                        }
                    }
                    ServerMessage::Changed {collection, id, fields, cleared} => {
                        let sub = &subs.lock().unwrap()[&collection];
                        let on_changed = sub.on_changed.clone();
                        if let Some(on_changed) = on_changed {
                            on_changed(collection, id, fields, cleared);
                        }
                    }
                    ServerMessage::Removed {collection, id} => {
                        let sub = &subs.lock().unwrap()[&collection];
                        let on_removed = sub.on_removed.clone();
                        if let Some(on_removed) = on_removed {
                            on_removed(collection, id);
                        }
                    }
                    ServerMessage::Ready {subs} => {
                        for id in subs.iter() {
                            sub_event.notify(id.clone(), ServerMessage::Ready {subs: vec![id.clone()]});
                        }

                    }
                    ServerMessage::Updated {methods: _} => {

                    }
                    _ => {
                        println!("invalid message: {:?}", message);
                    }
                }
            }

        })
    }

    fn alive_loop(alive: Arc<AtomicBool>, ws_tx_chan: mpsc::Sender<ws::OwnedMessage>, pong_event: Arc<Event<ServerMessage>>) -> JoinHandle<()> {
        thread::spawn(move || {
            let ping_id = 0;
            loop {
                if !alive.load(Ordering::SeqCst) {
                    return;
                }
                let ping_msg = ClientMessage::Ping {id: Some(ping_id.to_string())};
                if let Err(_) = ws_tx_chan.send(ws::OwnedMessage::Text(serde_json::to_string(&ping_msg).unwrap())) {
                    alive.store(false, Ordering::SeqCst);
                    return;
                }
                let _ = pong_event.wait(&ping_id.to_string()[..], Duration::from_millis(1000)).unwrap();
                thread::sleep(Duration::from_millis(1000));
            }
        })
    }

    pub fn subscribe(&mut self, collection: &str, timeout: Duration,
                     on_added: Option<fn(String, String, Option<Value>)>,
                     on_changed: Option<fn(String, String, Option<Value>, Option<Value>)>,
                     on_removed: Option<fn(String, String)>) -> Result<(), DDPError> {
        let sub_id = self.sub_id.to_string();
        self.sub_id += 1;
        let message = ClientMessage::Sub {
            id: sub_id.clone(),
            name: collection.to_string(),
            params: Option::None,
        };
        let sub_queue = Arc::new(Mutex::new(VecDeque::new()));
        let sub_cond = Arc::new(Condvar::new());
        let sub_th = SubScription::handle_process(sub_queue.clone(), sub_cond.clone());
        let sub = SubScription {
            id: sub_id.clone(),
            on_added,
            on_changed,
            on_removed,
            cond: sub_cond,
            queue: sub_queue,
            th: sub_th,
        };
        self.subs.lock().unwrap().insert(collection.to_string(), sub);
        self.send_message(serde_json::to_string(&message).unwrap())?;
        let response = self.sub_event.wait(&sub_id[..], timeout)?;
        match response {
            ServerMessage::Ready {subs: _} => {
                return Ok(());
            }
            ServerMessage::NoSub {id: _, error} => {
                return Err(DDPError::NoSub(serde_json::to_string(&error).unwrap()));
            }
            _ => {return Ok(());}
        }
    }

    pub fn unsubscribe(&mut self, collection: &str) {
        if let Some(sub) = self.subs.lock().unwrap().remove(collection) {
            if sub.id == collection {
                let message = ClientMessage::UnSub {
                    id: sub.id,
                };
                self.send_message(serde_json::to_string(&message).unwrap()).unwrap();
            }
        }

    }

    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Option<Value>, DDPError> {
        let method_id = self.method_id.to_string();
        self.method_id += 1;
        let message = ClientMessage::Method {
            method: method.to_string(),
            params: Some(params),
            id: method_id.clone(),
            randomSeed: Some(Value::String("0".to_string())),
        };
        self.send_message(serde_json::to_string(&message).unwrap())?;
        let msg = self.result_event.wait(&method_id[..], timeout)?;
        let msg = match msg {
            ServerMessage::Result{id: _, error, result} => {
                if let Some(error) = error {
                    return Err(DDPError::MethodError(serde_json::to_string(&error).unwrap()));
                }
                result
            },
            _ => {
                return Err(DDPError::InvalidMessage("invalid ddp message".to_string()));
            }
        };
        return Ok(msg);
    }

    pub fn is_alive(&self) -> bool {
        return self.alive.load(Ordering::SeqCst);
    }
}