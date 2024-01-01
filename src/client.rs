use websocket::client::ClientBuilder;
use websocket as ws;
use std::sync::mpsc;
use websocket::stream::sync::TcpStream;
use std::thread::JoinHandle;
use std::vec::Vec;
use std::sync::{Arc, Mutex};
use std::time::{Duration};
use std::thread;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use serde_json::Value;
use crate::message::{ServerMessage, ClientMessage};
use crate::error::DDPError;


/// Used to handle subscription message.
struct SubScription {
    id: String,
    sender: mpsc::Sender<ServerMessage>,
}

impl SubScription {

    pub fn new(id: String, handle: fn(ServerMessage)) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let msg = receiver.recv().unwrap();
                handle(msg);
            }
        });

        Self {
            id: id,
            sender: sender,
        }
    }
}

/// Create DDP client to communicate with Meteor DDP server.
///
/// ```rust,no_run
/// use rsddp::client::DDPClient;
/// let client = DDPClient::connect("ws://127.0.0.1:18001", Duration::from_millis(1000)).unwrap();
/// ```
pub struct DDPClient {
    wstx_chan: mpsc::Sender<ws::OwnedMessage>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ServerMessage>>>>,
    threads: Vec<JoinHandle<()>>,
    session: String,
    ids: Arc<AtomicU32>,
    alive: Arc<AtomicBool>,
    subs: Arc<Mutex<HashMap<String, SubScription>>>,
}

impl Drop for DDPClient {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.wstx_chan.send(ws::OwnedMessage::Close(None));
        while let Some(th) = self.threads.pop() {
            th.join().unwrap();
        }
    }
}

impl DDPClient {
    /// Connect server with endpoint string.
    pub fn connect(endpoint: &str) -> Result<Self, DDPError> {
        let client = ClientBuilder::new(endpoint)
                .map_err(|e| DDPError::UrlError(e.to_string()))?
                .connect_insecure().map_err(|e| DDPError::ConnectError(e.to_string()))?;
        let (mut ws_receiver, mut ws_sender) = client.split().unwrap();
        let ws_session = DDPClient::connect_with_ws(&mut ws_sender, &mut ws_receiver)?;
        let (wstx_chan, wsrx_chan) = mpsc::channel();
        let pending : Arc<Mutex<HashMap<String, oneshot::Sender<ServerMessage>>>> = Arc::new(Mutex::new(HashMap::new()));
        let ids: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
        let threads = Vec::new();
        let mut client = Self {
            pending: pending,
            threads: threads,
            session: ws_session,
            wstx_chan: wstx_chan,
            ids: ids,
            alive: Arc::new(AtomicBool::new(true)),
            subs: Arc::new(Mutex::new(HashMap::new())),
        };

        let send_th = DDPClient::ws_send_loop(wsrx_chan, ws_sender);
        client.threads.push(send_th);
        let recv_th = DDPClient::ws_recv_loop(&client, ws_receiver);
        client.threads.push(recv_th);
        let _ = DDPClient::alive_loop(&client);

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
                return Err(DDPError::ConnectError("the server and client are not compatible".to_string()));
            }
            let message = ws::OwnedMessage::Text(serde_json::to_string(&connect_message).unwrap());
            ws_sender.send_message(&message).map_err(|e| DDPError::SendError(e.to_string()))?;
            if let ws::OwnedMessage::Text(message) = ws_receiver.recv_message().map_err(|e| DDPError::RecvError(e.to_string()))? {
                let message = serde_json::from_str(&message[..]).map_err(|e| DDPError::MessageError(e.to_string()))?;
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
                            return Err(DDPError::ConnectError(format!("server does not support current client, suggested protocol version is {}", version)));
                        }
    
                    }
                    _ => {
                        return Err(DDPError::MessageError(format!("invalid message: {}", serde_json::to_string(&message).unwrap())));
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
        let subs_table = self.subs.clone();
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
                            resp_sender.send(ServerMessage::Result {id, error, result}).unwrap();
                        }
                    }
                    ServerMessage::Pong {id} => {
                        let mut pending = pending.lock().unwrap();
                        if let Some(resp_sender) = pending.remove(&(id.clone().unwrap())) {
                            resp_sender.send(ServerMessage::Pong {id}).unwrap();
                        }
                    }
                    ServerMessage::NoSub {id, error} => {
                        let sub = &subs_table.lock().unwrap()[&id];
                        sub.sender.send(ServerMessage::NoSub {id, error}).unwrap();
                    }
                    ServerMessage::Added {collection, id, fields} => {
                        let subs = &subs_table.lock().unwrap();
                        for (_, sub) in subs.iter() {
                            if sub.id == id {
                                sub.sender.send(ServerMessage::Added {collection, id, fields}).unwrap();
                                break;
                            }
                        }
                        
                    }
                    ServerMessage::Changed {collection, id, fields, cleared} => {
                        let subs = &subs_table.lock().unwrap();
                        for (_, sub) in subs.iter() {
                            if sub.id == id {
                                sub.sender.send(ServerMessage::Changed {collection, id, fields, cleared}).unwrap();
                                break;
                            }
                        }
                    }
                    ServerMessage::Removed {collection, id} => {
                        let subs = &subs_table.lock().unwrap();
                        for (_, sub) in subs.iter() {
                            if sub.id == id {
                                sub.sender.send(ServerMessage::Removed {collection, id}).unwrap();
                                break;
                            }
                        } 
                    }
                    ServerMessage::Ready {subs} => {
                        for id in subs.iter() {
                            let sub = &subs_table.lock().unwrap()[id];
                            sub.sender.send(ServerMessage::Ready {subs: vec![id.clone()]}).unwrap();
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

    fn alive_loop(&self) -> JoinHandle<()> {
        let alive = self.alive.clone();
        let wstx_chan = self.wstx_chan.clone();
        let pending = self.pending.clone();
        let ids = self.ids.clone();
        thread::spawn(move || {
            loop {
                if !alive.load(Ordering::SeqCst) {
                    return;
                }
                
                let ping_id: String = ids.fetch_add(1, Ordering::SeqCst).to_string();
                let ping_msg = ClientMessage::Ping {id: Some(ping_id.clone())};
                let (resp_sender, resp_receiver) = oneshot::channel();
                {
                    let mut pending = pending.lock().unwrap();
                    pending.insert(ping_id.clone(), resp_sender);
                }
                let message = ws::OwnedMessage::Text(serde_json::to_string(&ping_msg).unwrap());
                wstx_chan.send(message).unwrap();
                if let Ok(resp) = resp_receiver.recv_timeout(Duration::from_millis(1000)) {
                    if let ServerMessage::Pong {id} = resp {
                        if id != Some(ping_id) {
                            alive.store(false, Ordering::SeqCst);
                            return;
                        }
                    }
                }
                thread::sleep(Duration::from_millis(1000));
            }
        })
    }

    /// call remote method in server.
    ///
    /// ```rust,no_run
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use websocket::client::DDPClient;
    ///
    /// let client = DDPClient::connect("ws://127.0.0.1:8000").unwrap();
    /// let result = client.call("hello", json!(["hello", "world"]), Duration::from_millis(1000)).unwrap();
    /// println!("{:?}", result);
    /// ```
    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Option<Value>, DDPError> {
        let method_id: String = self.ids.fetch_add(1, Ordering::SeqCst).to_string();
        let message = ClientMessage::Method {
            method: method.to_string(),
            params: Some(params),
            id: method_id.clone(),
            random_seed: None,
        };
        let (resp_sender, resp_receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(method_id, resp_sender);
        }
        let message = serde_json::to_string(&message).unwrap();
        let message = ws::OwnedMessage::Text(message);
        self.wstx_chan.send(message).unwrap();
        if let Ok(resp) = resp_receiver.recv_timeout(timeout) {
            let resp = match resp {
                ServerMessage::Result {id: _, error, result} => {
                    if let Some(error) = error {
                        return Err(DDPError::MethodError(serde_json::to_string(&error).unwrap()));
                    }
                    result
                },
                _ => {
                    return Err(DDPError::MessageError("Invalid ddp message".to_string()));
                }
            };
            return Ok(resp);
        }
        return Err(DDPError::MessageError("Invalid ddp message".to_string()));
    }

    /// Sub messages in server side.
    ///
    /// ```rust,no_run
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use websocket::client::DDPClient;
    /// use rsddp::message::ServerMessage;
    ///
    /// fn on_sub_data(message: ServerMessage) {
    ///    println!("on_added: {}", serde_json::to_string(&message).unwrap());
    /// }
    /// let client = DDPClient::connect("ws://127.0.0.1:8000").unwrap();
    /// let _  = client.subscribe("1".to_string(), "topic".to_string(), on_sub_data);
    /// ```
    pub fn subscribe(&mut self, id: String,
                     collection: String,
                     handle: fn(ServerMessage)) -> Result<(), DDPError> {
        let sub = SubScription::new(id.clone(), handle);
        let mut subs = self.subs.lock().unwrap();
        subs.insert(id.clone(), sub);
        let message = ClientMessage::Sub {id, name: collection, params: None};
        let message = serde_json::to_string(&message).unwrap();
        self.wstx_chan.send(ws::OwnedMessage::Text(message)).unwrap();
        return Ok(());
    }

    /// You need unsubscribe message which you have subscribed with `subscribe` method
    ///
    /// ```rust
    /// client.unsubscribe("1".to_string());
    /// ```
    pub fn unsubscribe(&mut self, id: &String) {
        let mut subs = self.subs.lock().unwrap();
        subs.remove(id);
    }

    pub fn is_alive(&self) -> bool {
        return self.alive.load(Ordering::SeqCst);
    }
}