use websocket::client::ClientBuilder;
use websocket as ws;
use std::sync::mpsc;
use websocket::stream::sync::TcpStream;
use std::thread::JoinHandle;
use std::vec::Vec;
use std::sync::{Arc};
use std::time::{Duration, Instant};
use std::thread::{self, spawn, sleep};
use std::sync::atomic::{AtomicBool, Ordering};
use serde_json::Value;
use crate::event::Event;
use crate::message::{ServerMessage, ClientMessage};
use crate::error::DDPError;


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
    alive: Arc<AtomicBool>,
}

impl Drop for DDPClient {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
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

        let alive_ws_tx = ws_tx.clone();
        let mut threads = Vec::new();
        let conn_event = Arc::new(Event::<ServerMessage>::new());
        let pong_event = Arc::new(Event::<ServerMessage>::new());
        let pong_event_alive = pong_event.clone();
        let sub_event = Arc::new(Event::<ServerMessage>::new());
        let sub_event_clone = sub_event.clone();
        let result_event = Arc::new(Event::<ServerMessage>::new());
        let updated_event = Arc::new(Event::<ServerMessage>::new());
        let updated_event_clone = updated_event.clone();
        let send_th = DDPClient::ws_send_loop(ws_rx, sender);
        threads.push(send_th);
        let recv_th = DDPClient::ws_recv_loop(receiver, ws_tx.clone(), conn_event.clone(), result_event.clone(), pong_event.clone());
        threads.push(recv_th);
        let is_alive = Arc::new(AtomicBool::new(true));
        let mut client = Self {
            ws_tx: ws_tx,
            threads: threads,
            conn_event: conn_event,
            pong_event: pong_event,
            sub_event: sub_event_clone,
            result_event: result_event,
            updated_event: updated_event_clone,
            session: "".to_string(),
            method_id: 0,
            alive: is_alive.clone(),
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
            let message = client.conn_event.wait_timeout("", timeout)?;
            match message {
                ServerMessage::Connected {session} => {
                    client.session = session;
                    break;
                }
                ServerMessage::Failed {version} => {
                    connect_message = ClientMessage::Connect {
                        session: "".to_string(),
                        version: ddp_versions[v_index].clone(),
                        support: ddp_versions.clone(),
                    };
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

    fn send_message(&self, msg: ws::OwnedMessage) -> Result<(), DDPError> {
        match self.ws_tx.send(msg) {
            Ok(()) => Ok(()),
            Err(_) => Err(DDPError::SendFailed("send message failed".to_string()))
        }
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
                    mut ws_tx_chan: mpsc::Sender<ws::OwnedMessage>,
                    conn_event: Arc<Event<ServerMessage>>,
                    result_event: Arc<Event<ServerMessage>>,
                    pong_event: Arc<Event<ServerMessage>>) -> JoinHandle<()> {
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
                    _ => {
                        println!("invalid message: {:?}", message);
                    }
                }
            }

        })
    }

    fn alive_loop(alive: Arc<AtomicBool>, ws_tx_chan: mpsc::Sender<ws::OwnedMessage>, pong_event: Arc<Event<ServerMessage>>) -> JoinHandle<()> {
        thread::spawn(move || {
            let mut ping_id = 0;
            loop {
                if !alive.load(Ordering::SeqCst) {
                    return;
                }
                let ping_msg = ClientMessage::Ping {id: Some(ping_id.to_string())};
                if let Err(_) = ws_tx_chan.send(ws::OwnedMessage::Text(serde_json::to_string(&ping_msg).unwrap())) {
                    alive.store(false, Ordering::SeqCst);
                    return;
                }
                let _ = pong_event.wait_timeout(&ping_id.to_string()[..], Duration::from_millis(1000)).unwrap();
                thread::sleep(Duration::from_millis(1000));
            }
        })
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
        self.send_message(ws::OwnedMessage::Text(serde_json::to_string(&message).unwrap()))?;
        let msg = self.result_event.wait_timeout(&method_id[..], timeout)?;
        let msg = match msg {
            ServerMessage::Result{id, error, result} => {
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