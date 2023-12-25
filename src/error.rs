

#[derive(Debug)]
pub enum DDPError {
    UrlError(String),
    WSConnError(String),
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