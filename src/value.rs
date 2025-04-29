use core::fmt;
use std::fmt::Display;

use musix::MusicError;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Text(String),
    Number(f64),
    Data(Vec<u8>),
    Error(MusicError),
    Unknown(),
}

impl Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Text(s) => f.write_str(s.as_str())?,
            Value::Number(n) => write!(f, "{n:02}")?,
            Value::Error(e) => write!(f, "{e}")?,
            Value::Data(_) => write!(f, "Data")?,
            Value::Unknown() => write!(f, "???")?,
        }
        Ok(())
    }
}

impl From<f64> for Value {
    fn from(item: f64) -> Self {
        Value::Number(item)
    }
}

impl From<i32> for Value {
    fn from(item: i32) -> Self {
        Value::Number(f64::from(item))
    }
}

impl From<String> for Value {
    fn from(item: String) -> Self {
        Value::Text(item)
    }
}

impl From<&str> for Value {
    fn from(item: &str) -> Self {
        Value::Text(item.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(item: Vec<u8>) -> Self {
        Value::Data(item)
    }
}

impl From<MusicError> for Value {
    fn from(item: MusicError) -> Self {
        Value::Error(item)
    }
}
