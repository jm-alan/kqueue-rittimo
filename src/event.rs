use std::{io, os::fd::RawFd};

use kqueue_sys::{EventFilter, FilterFlag, kevent};
use libc::pid_t;

use crate::{EventData, Ident, Proc, Vnode, Watcher, find_file_ident, os::vnode};

/// An event from a `Watcher` object.
///
/// An event contains both the a signifier of the watched object that triggered
/// the event, as well as any event-specific. See the `EventData` enum for info
/// on what event-specific data is returned for each event.
#[derive(Debug)]
pub struct Event {
  /// The watched resource that triggered the event
  pub ident: Ident,

  /// Any event-specific data returned with the event.
  pub data: EventData,
}

// OS specific
// TODO: Events can have more than one filter flag
impl Event {
  #[doc(hidden)]
  pub fn new(ev: kevent, watcher: &Watcher) -> Event {
    let data = match ev.filter {
      EventFilter::EVFILT_READ => EventData::ReadReady(ev.data as usize),
      EventFilter::EVFILT_WRITE => EventData::WriteReady(ev.data as usize),
      EventFilter::EVFILT_SIGNAL => EventData::Signal(ev.data as usize),
      EventFilter::EVFILT_TIMER => EventData::Timer(ev.data as usize),
      EventFilter::EVFILT_PROC => {
        let inner = if ev.fflags.contains(FilterFlag::NOTE_EXIT) {
          Proc::Exit(ev.data as usize)
        } else if ev.fflags.contains(FilterFlag::NOTE_FORK) {
          Proc::Fork
        } else if ev.fflags.contains(FilterFlag::NOTE_EXEC) {
          Proc::Exec
        } else if ev.fflags.contains(FilterFlag::NOTE_TRACK) {
          Proc::Track(ev.data as libc::pid_t)
        } else if ev.fflags.contains(FilterFlag::NOTE_CHILD) {
          Proc::Child(ev.data as libc::pid_t)
        } else {
          panic!("proc filterflag not supported: {0:?}", ev.fflags)
        };

        EventData::Proc(inner)
      },
      EventFilter::EVFILT_VNODE => {
        let inner = if ev.fflags.contains(FilterFlag::NOTE_DELETE) {
          Vnode::Delete
        } else if ev.fflags.contains(FilterFlag::NOTE_WRITE) {
          Vnode::Write
        } else if ev.fflags.contains(FilterFlag::NOTE_EXTEND) {
          Vnode::Extend
        } else if ev.fflags.contains(FilterFlag::NOTE_ATTRIB) {
          Vnode::Attrib
        } else if ev.fflags.contains(FilterFlag::NOTE_LINK) {
          Vnode::Link
        } else if ev.fflags.contains(FilterFlag::NOTE_RENAME) {
          Vnode::Rename
        } else if ev.fflags.contains(FilterFlag::NOTE_REVOKE) {
          Vnode::Revoke
        } else {
          // This handles any filter flags that are OS-specific
          vnode::handle_vnode_extras(ev.fflags)
        };

        EventData::Vnode(inner)
      },
      _ => panic!("eventfilter not supported: {0:?}", ev.filter),
    };

    let ident = match ev.filter {
      EventFilter::EVFILT_READ => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_WRITE => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_VNODE => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_SIGNAL => Ident::Signal(ev.ident as i32),
      EventFilter::EVFILT_TIMER => Ident::Timer(ev.ident as u64),
      EventFilter::EVFILT_PROC => Ident::Pid(ev.ident as pid_t),
      _ => panic!("not supported"),
    };

    Event { ident, data }
  }

  #[doc(hidden)]
  pub fn from_error(ev: kevent, watcher: &Watcher) -> Event {
    let ident = match ev.filter {
      EventFilter::EVFILT_READ => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_WRITE => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_VNODE => find_file_ident(watcher, ev.ident as RawFd).unwrap(),
      EventFilter::EVFILT_SIGNAL => Ident::Signal(ev.ident as i32),
      EventFilter::EVFILT_TIMER => Ident::Timer(ev.ident as u64),
      EventFilter::EVFILT_PROC => Ident::Pid(ev.ident as pid_t),
      _ => panic!("not supported"),
    };

    Event {
      data: EventData::Error(io::Error::last_os_error()),
      ident,
    }
  }

  #[doc(hidden)]
  pub fn is_err(&self) -> bool {
    matches!(self.data, EventData::Error(_))
  }
}
