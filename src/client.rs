use websocket::client::ClientBuilder;
use websocket as ws;
use std::sync::mpsc;
use websocket::stream::sync::TcpStream;
use std::thread::JoinHandle;
use std::vec::Vec;
use std::sync::{Arc, Mutex, Condvar};
use std::time::{Duration};
use std::thread::{self, spawn};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use serde_json::Value;
use crate::message::{ServerMessage, ClientMessage};
use crate::error::DDPError;


// struct SubScription {
//     id: String,
//     on_added: Option<fn(String, String, Option<Value>)>,
//     on_changed: Option<fn(String, String, Option<Value>, Option<Value>)>,
//     on_removed: Option<fn(String, String)>,
//     queue: Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>,
//     cond: Arc<Condvar>,
//     th: JoinHandle<()>,
// }

// impl SubScription {
//     fn add(&mut self, collection: String, id: String, fields: Option<Value>) {
//         let mut queue = self.queue.lock().unwrap();
//         let on_added = self.on_added.clone();
//         queue.push_back(Box::new(move || {
//             if let Some(on_added) = on_added {
//                 on_added(collection, id, fields);
//             }
//         }) );
//         self.cond.notify_all();
//     }

//     fn handle_process(queue: Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>, cond: Arc<Condvar>) -> JoinHandle<()> {
//         spawn(move || {
//             loop {
//                 let mut msgs = queue.lock().unwrap();
//                 while msgs.len() <= 0 {
//                     msgs = cond.wait(msgs).unwrap();
//                 }
//                 if let Some(handle) = msgs.pop_back() {
//                     handle();
//                 }
//             }
//         })
//     }
// }

pub struct DDPClient {
    wstx_chan: mpsc::Sender<ws::OwnedMessage>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ServerMessage>>>>,
    threads: Vec<JoinHandle<()>>,
    session: String,
    ids: Arc<AtomicU32>,
    alive: Arc<AtomicBool>
}

impl Drop for DDPClient {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.wstx_chan.send(ws::OwnedMessage::Close(None));
        println!("---------------------");
        while let Some(th) = self.threads.pop() {
            th.join().unwrap();
        }
    }
}

impl DDPClient {
    pub fn connect(endpoint: &str, timeout: Duration) -> Result<Self, DDPError> {
        let client = ClientBuilder::new(endpoint).unwrap().connect_insecure().unwrap();
        let (mut ws_receiver, mut ws_sender) = client.split().unwrap();
        let ws_session = DDPClient::connect_with_ws(&mut ws_sender, &mut ws_receiver)?;
        let (wstx_chan, wsrx_chan) = mpsc::channel();
        let pending : Arc<Mutex<HashMap<String, oneshot::Sender<ServerMessage>>>> = Arc::new(Mutex::new(HashMap::new()));
        let ids: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
        let mut threads = Vec::new();
        let mut client = Self {
            pending: pending,
            threads: threads,
            session: ws_session,
            wstx_chan: wstx_chan,
            ids: ids,
            alive: Arc::new(AtomicBool::new(true)),
        };

        let send_th = DDPClient::ws_send_loop(wsrx_chan, ws_sender);
        client.threads.push(send_th);
        let recv_th = DDPClient::ws_recv_loop(&client, ws_receiver);
        client.threads.push(recv_th);
        let alive_th = DDPClient::alive_loop(&client);
        client.threads.push(alive_th);

        return Ok(client);
    }

    fn connect_with_ws(ws_sender: &mut ws::sender::Writer<TcpStream>, ws_receiver: &mut ws::receiver::Reader<TcpStream>) -> Result<String, DDPError> {
        let mut v_index = 0;
        let ddp_versions: Vec<String> = vec!["1".to_string(), "pre2".to_string(), "pre1".to_string()]; 

        let mut connect_message = ClientMessage::Connect {
            session: "".to_string(),
            version: ddp_versions[v_index].clone(),
            support: ddp_versions.clone(),
        };
        let ws_session: String;
        'loop1: loop {
            v_index += 1;
            if v_index >= ddp_versions.len() {
                return Err(DDPError::NotMatching("no matching version".to_string()));
            }
            let message = ws::OwnedMessage::Text(serde_json::to_string(&connect_message).unwrap());
            ws_sender.send_message(&message).unwrap();
            if let ws::OwnedMessage::Text(message) = ws_receiver.recv_message().unwrap() {
                let message = serde_json::from_str(&message[..]).unwrap();
                match message {
                    ServerMessage::Connected {session} => {
                        ws_session = session;
                        break 'loop1;
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
        }
        return Ok(ws_session);
    }

    fn ws_send_loop(wsrx_chan: mpsc::Receiver<ws::OwnedMessage>, mut ws_sender: ws::sender::Writer<TcpStream>) -> JoinHandle<()> {
        thread::spawn(move || {
            loop {
                let message = match wsrx_chan.recv() {
                    Ok(m) => m,
                    Err(e) => {
                        println!("send loop: {:?}", e);
                        return;
                    }
                };
                match message {
                    ws::OwnedMessage::Close(_) => {
                        let _ = ws_sender.send_message(&message);
                        println!("----exit send loop");
                        return;
                    }
                    _ => {
                        ws_sender.send_message(&message).unwrap();
                    }
                }
            }
        })
    }

    fn ws_recv_loop(&self, mut ws_recver: ws::receiver::Reader<TcpStream>) -> JoinHandle<()> {
        let pending = self.pending.clone();
        let wstx_chan = self.wstx_chan.clone();
        thread::spawn(move || {
            for message in ws_recver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Receive loop: {:?}", e);
                        return;
                    }
                };
                let message: ServerMessage = match message {
                    ws::OwnedMessage::Text(m) => serde_json::from_str(&m[..]).unwrap(),
                    ws::OwnedMessage::Close(_) => {
                        println!("-----------------------recv loop");
                        return;
                    }
                    ws::OwnedMessage::Ping(data) => {
                        match wstx_chan.send(ws::OwnedMessage::Pong(data)) {
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
                    ServerMessage::Result {id, error, result} => {
                        let mut pending = pending.lock().unwrap();
                        if let Some(resp_sender) = pending.remove(&id) {
                            resp_sender.send(ServerMessage::Result {id, error, result});
                        }
                    }
                    ServerMessage::Pong {id} => {
                        let mut pending = pending.lock().unwrap();
                        if let Some(resp_sender) = pending.remove(&(id.clone().unwrap())) {
                            resp_sender.send(ServerMessage::Pong {id});
                        }
                    }
                    // ServerMessage::NoSub {id, error} => {
                    //     sub_event.notify(id.to_string(), ServerMessage::NoSub {id, error});
                    // }
                    // ServerMessage::Added {collection, id, fields} => {
                    //     let sub = &subs.lock().unwrap()[&collection];
                    //     let on_added = sub.on_added.clone();
                    //     if let Some(on_added) = on_added {
                    //         on_added(collection, id, fields);
                    //     }
                    // }
                    // ServerMessage::Changed {collection, id, fields, cleared} => {
                    //     let sub = &subs.lock().unwrap()[&collection];
                    //     let on_changed = sub.on_changed.clone();
                    //     if let Some(on_changed) = on_changed {
                    //         on_changed(collection, id, fields, cleared);
                    //     }
                    // }
                    // ServerMessage::Removed {collection, id} => {
                    //     let sub = &subs.lock().unwrap()[&collection];
                    //     let on_removed = sub.on_removed.clone();
                    //     if let Some(on_removed) = on_removed {
                    //         on_removed(collection, id);
                    //     }
                    // }
                    // ServerMessage::Ready {subs} => {
                    //     for id in subs.iter() {
                    //         sub_event.notify(id.clone(), ServerMessage::Ready {subs: vec![id.clone()]});
                    //     }

                    // }
                    // ServerMessage::Updated {methods: _} => {

                    // }
                    _ => {
                        println!("invalid message: {:?}", message);
                    }
                }
            }
        })
    }

    fn alive_loop(&self) -> JoinHandle<()> {
        let alive = self.alive.clone();
        let wstx_chan = self.wstx_chan.clone();
        let pending = self.pending.clone();
        let ids = self.ids.clone();
        thread::spawn(move || {
            loop {
                if !alive.load(Ordering::SeqCst) {
                    println!("--------------exit alive");
                    return;
                }
                
                let ping_id: String = ids.fetch_add(1, Ordering::SeqCst).to_string();
                let ping_msg = ClientMessage::Ping {id: Some(ping_id.clone())};
                let (resp_sender, resp_receiver) = oneshot::channel();
                {
                    let mut pending = pending.lock().unwrap();
                    pending.insert(ping_id, resp_sender);
                }
                let message = ws::OwnedMessage::Text(serde_json::to_string(&ping_msg).unwrap());
                wstx_chan.send(message);
                if let Ok(resp) = resp_receiver.recv_timeout(Duration::from_millis(1000)) {

                }
                thread::sleep(Duration::from_millis(1000));
            }
        })
    }

    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Option<Value>, DDPError> {
        let method_id: String = self.ids.fetch_add(1, Ordering::SeqCst).to_string();
        let message = ClientMessage::Method {
            method: method.to_string(),
            params: Some(params),
            id: method_id.clone(),
            randomSeed: Some(Value::String("0".to_string())),
        };
        let (resp_sender, resp_receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(method_id, resp_sender);
        }
        let message = serde_json::to_string(&message).unwrap();
        let message = ws::OwnedMessage::Text(message);
        self.wstx_chan.send(message);
        if let Ok(resp) = resp_receiver.recv_timeout(timeout) {
            let resp = match resp {
                ServerMessage::Result {id: _, error, result} => {
                    if let Some(error) = error {
                        return Err(DDPError::MethodError(serde_json::to_string(&error).unwrap()));
                    }
                    result
                },
                _ => {
                    return Err(DDPError::InvalidMessage("Invalid ddp message".to_string()));
                }
            };
            return Ok(resp);
        }
        return Err(DDPError::InvalidMessage("Invalid ddp message".to_string()));
    }

    pub fn is_alive(&self) -> bool {
        return self.alive.load(Ordering::SeqCst);
    }
}