use anyhow::Result;
use core::fmt;
use crossterm::{Command, event::KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetReverse(pub bool);

impl Command for SetReverse {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.0 {
            write!(f, "\x1b[7m")
        } else {
            write!(f, "\x1b[27m")
        }
    }
}

#[derive(Debug)]
pub enum MaybeCommand<T: Command> {
    Set(T),
    None,
}

impl<T: Command> Command for MaybeCommand<T> {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        if let MaybeCommand::Set(cmd) = self {
            cmd.write_ansi(f)
        } else {
            Ok(())
        }
    }
}

pub trait TextComponent<T = ()> {
    type UiState;
    type Return;
    fn handle_key(&mut self, _state: &mut Self::UiState, _key: KeyEvent) -> Result<Self::Return>;
    fn draw(&self, state: &mut Self::UiState) -> Result<()>;
}
