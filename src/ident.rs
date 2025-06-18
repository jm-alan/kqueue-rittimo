use std::{fs::File, os::fd::RawFd, time::Duration};

use libc::pid_t;

/// The watched object that fired the event
#[derive(Debug)]
pub enum Ident {
  Filename(File, RawFd, String),
  Fd(RawFd),
  Pid(pid_t),
  Signal(i32),
  Timer(usize, Duration),
}

// We don't have enough information to turn a `usize` into
// an `Ident`, so we only implement `Into<usize>` here.
#[allow(clippy::from_over_into)]
impl Into<usize> for Ident {
  fn into(self) -> usize {
    match self {
      Ident::Filename(_, fd, _) => fd as usize,
      Ident::Fd(fd) => fd as usize,
      Ident::Pid(pid) => pid as usize,
      Ident::Signal(sig) => sig as usize,
      Ident::Timer(timer, _) => timer,
    }
  }
}

impl PartialEq for Ident {
  fn eq(&self, other: &Ident) -> bool {
    match *self {
      Ident::Filename(_, _, ref name) => {
        if let Ident::Filename(_, _, ref othername) = *other {
          name == othername
        } else {
          false
        }
      },
      _ => self.as_usize() == other.as_usize(),
    }
  }
}

impl Eq for Ident {}

impl Ident {
  pub(crate) fn as_usize(&self) -> usize {
    match *self {
      Ident::Filename(_, fd, _) => fd as usize,
      Ident::Fd(fd) => fd as usize,
      Ident::Pid(pid) => pid as usize,
      Ident::Signal(sig) => sig as usize,
      Ident::Timer(timer, _) => timer as usize,
    }
  }
}
