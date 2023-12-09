

#[derive(Debug)]
pub enum DDPError {
    Timeout(String),
    InvalidMessage(String),
    SendFailed(String),
    ConnFailed(String),
    NotMatching(String),
    MethodError(String),
    Other(String),
}