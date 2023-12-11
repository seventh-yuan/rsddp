extern crate rsddp;

use std::thread;
use serde_json::json;
use std::time::Duration;
use rsddp::client::DDPClient;


fn main() {
    let mut client = DDPClient::connect("ws://127.0.0.1:18000", Duration::from_millis(1000)).unwrap();
    let result = client.call("hello", json!([{"args": ["hello"], "kargs": {}}]), Duration::from_millis(1000)).unwrap();
    println!("{:?}", result);
    thread::sleep(Duration::from_millis(2000));

}

