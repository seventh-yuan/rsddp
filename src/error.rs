
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum DDPError {
    UrlError(String),
    ConnectError(String),
    SendError(String),
    RecvError(String),
    MessageError(String),
    MethodError(String),
    Other(String),
}
