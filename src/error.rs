

#[derive(Debug)]
pub enum DDPError {
    Timeout(String),
    InvalidMessage(String),
    SendFailed(String),
    ConnFailed(String),
    NotMatching(String),
    MethodError(String),
    NoSub(String),
    NotSupport(String),
    Other(String),
}