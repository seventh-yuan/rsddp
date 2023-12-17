

#[derive(Debug)]
pub enum DDPError {
    Timeout(String),
    InvalidMessage(String),
    SendFailed(String),
    ConnFailed(String),
    NotMatching(String),
    MethodError(String),
    NoSub(String),
    Other(String),
}