extern crate rsddp;

use std::thread;
use serde_json::{json, Value};
use std::time::Duration;
use rsddp::client::DDPClient;


// fn on_added(collection: String, id: String, fields: Option<Value>) {
//     println!("on_added: {}", serde_json::to_string(&fields).unwrap());
// }



fn main() {
    let on_added = |collection: String, id: String, fields: Option<Value>| {
        println!("on_added: {}", serde_json::to_string(&fields).unwrap());
    };
    let mut client = DDPClient::connect("ws://127.0.0.1:18001", Duration::from_millis(1000)).unwrap();
    let result = client.call("hello", json!([{"args": ["hello"], "kargs": {}}]), Duration::from_millis(1000)).unwrap();
    println!("{:?}", result);
    // let _  = client.subscribe("posts", Duration::from_millis(1000), Some(on_added), None, None);
    // let result = client.call("demo.set_post", json!([{"args": ["hello world!"], "kargs": {}}]), Duration::from_millis(1000)).unwrap();
    // thread::sleep(Duration::from_millis(5000));
}

