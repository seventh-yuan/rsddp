
use serde::{Serialize, Deserialize};
use serde_json::Value;
use std::vec::Vec;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "msg")]
pub enum ClientMessage {
    Connect {
        session: String,
        version: String,
        support: Vec<String>,
    },
    Ping {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>
    },
    Sub {
        id: String,
        name: String,
        params: Option<Vec<Value>>,
    },
    UnSub {
        id: String,
    },
    Method {
        method: String,
        params: Option<Value>,
        id: String,
        randomSeed: Option<Value>
    },

}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "msg")]
pub enum ServerMessage {
    Connected {
        session: String,
    },
    Failed {
        version: String,
    },
    Pong {
        id: Option<String>
    },
    NoSub {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Value>
    },
    Added {
        collection: String,
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<Value>
    },
    Changed {
        collection: String,
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cleared: Option<Value>,
    },
    Removed {
        collection: String,
        id: String,
    },
    Ready {
        subs: Vec<String>,
    },
    AddedBefore {
        collection: String,
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<Value>,
        before: Option<String>,
    },
    MovedBefore {
        collection: String,
        id: String,
        before: Option<String>
    },
    Result {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
    },
    Updated {
        methods: Vec<String>
    }
}