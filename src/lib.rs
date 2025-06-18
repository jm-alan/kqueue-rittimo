use kqueue_sys::{kevent, kqueue};
use libc::uintptr_t;
use std::collections::HashSet;
use std::fmt::Debug;
use std::fs::File;
use std::io::{Error, Result};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::ptr;
use std::time::Duration;

pub use kqueue_sys::constants::*;

mod event;
mod ident;
mod os;
mod watched;
pub use event::Event;
pub use ident::Ident;
pub use watched::Watched;

mod time;
use crate::time::duration_to_timespec;

/// Watches one or more resources
///
/// These can be created with `Watcher::new()`. You can create as many
/// `Watcher`s as you want, and they can watch as many objects as you wish.
/// The objects do not need to be the same type.
///
/// Each `Watcher` is backed by a `kqueue(2)` queue. These resources are freed
/// on the `Watcher`s destruction. If the destructor cannot run for whatever
/// reason, the underlying kernel object will be leaked.
///
/// Files and file descriptors given to the `Watcher` are presumed to be owned
/// by the `Watcher`, and will be closed when they're removed from the `Watcher`
/// or on `Drop`. In a future version, the API will make this explicit via
/// `OwnedFd`s
#[derive(Debug)]
pub struct Watcher {
  watched: HashSet<Watched>,
  queue: RawFd,
  started: bool,
  opts: KqueueOpts,
}

/// Vnode events
///
/// These are OS-specific, and may not all be supported on your platform. Check
/// `kqueue(2)` for more information.
#[derive(Debug)]
#[non_exhaustive]
pub enum Vnode {
  /// The file was deleted
  Delete,

  /// The file received a write
  Write,

  /// The file was extended with `truncate(2)`
  Extend,

  /// The file was shrunk with `truncate(2)`
  Truncate,

  /// The attributes of the file were changed
  Attrib,

  /// The link count of the file was changed
  Link,

  /// The file was renamed
  Rename,

  /// Access to the file was revoked with `revoke(2)` or the fs was unmounted
  Revoke,

  /// File was opened by a process (FreeBSD-specific)
  Open,

  /// File was closed and the descriptor had write access (FreeBSD-specific)
  CloseWrite,

  /// File was closed and the descriptor had read access (FreeBSD-specific)
  Close,
}

/// Process events
///
/// These are OS-specific, and may not all be supported on your platform. Check
/// `kqueue(2)` for more information.
#[derive(Debug)]
pub enum Proc {
  /// The watched process exited with the returned exit code
  Exit(usize),

  /// The process called `fork(2)`
  Fork,

  /// The process called `exec(2)`
  Exec,

  /// The process called `fork(2)`, and returned the child pid.
  Track(libc::pid_t),

  /// The process called `fork(2)`, but we were not able to track the child
  Trackerr,

  /// The process called `fork(2)`, and returned the child pid.
  // TODO: this is FreeBSD-specific. We can probably convert this to `Track`.
  Child(libc::pid_t),
}

/// Event-specific data returned with the event.
///
/// Like much of this library, this is OS-specific. Check `kqueue(2)` for more
/// details on your target OS.
#[derive(Debug)]
pub enum EventData {
  /// Data relating to `Vnode` events
  Vnode(Vnode),

  /// Data relating to process events
  Proc(Proc),

  /// The returned number of bytes are ready for reading from the watched
  /// descriptor
  ReadReady(usize),

  /// The file is ready for writing. On some files (like sockets, pipes, etc),
  /// the number of bytes in the write buffer will be returned.
  WriteReady(usize),

  /// One of the watched signals fired. The number of times this signal was received
  /// is returned.
  Signal(usize),

  /// One of the watched timers fired. The number of times this timer fired
  /// is returned.
  Timer(usize),

  /// Some error was received
  Error(Error),
}

pub struct EventIter<'a> {
  watcher: &'a Watcher,
}

/// Options for a `Watcher`
#[derive(Debug)]
pub struct KqueueOpts {
  /// Clear state on watched objects
  clear: bool,
}

impl Default for KqueueOpts {
  /// Returns the default options for a `Watcher`
  ///
  /// `clear` is set to `true`
  fn default() -> KqueueOpts {
    KqueueOpts { clear: true }
  }
}

impl Watcher {
  /// Creates a new `Watcher`
  ///
  /// Creates a brand new `Watcher` with `KqueueOpts::default()`. Will return
  /// an `io::Error` if creation fails.
  pub fn new() -> Result<Watcher> {
    let queue = unsafe { kqueue() };

    if queue == -1 {
      Err(Error::last_os_error())
    } else {
      Ok(Watcher {
        watched: HashSet::new(),
        queue,
        started: false,
        opts: Default::default(),
      })
    }
  }

  /// Disables the `clear` flag on a `Watcher`. New events will no longer
  /// be added with the `EV_CLEAR` flag on `watch`.
  pub fn disable_clears(&mut self) -> &mut Self {
    self.opts.clear = false;
    self
  }

  /// Adds a `pid` to the `Watcher` to be watched
  pub fn add_pid(&mut self, pid: libc::pid_t, filter: EventFilter, flags: FilterFlag) {
    let watch = Watched {
      filter,
      flags,
      ident: Ident::Pid(pid),
    };

    if !self.watched.contains(&watch) {
      self.watched.insert(watch);
    }
  }

  /// Adds a file by filename to be watched
  ///
  /// **NB**: `kqueue(2)` is an `fd`-based API. If you add a filename with
  /// `add_filename`, internally we open it and pass the file descriptor to
  /// `kqueue(2)`. If the file is moved or deleted, and a new file is created
  /// with the same name, you will not receive new events for it without
  /// calling `add_filename` again.
  ///
  /// TODO: Adding new files requires calling `Watcher.watch` again
  pub fn add_filename<P: AsRef<Path>>(
    &mut self,
    filename: P,
    filter: EventFilter,
    flags: FilterFlag,
  ) -> Result<()> {
    let file = File::open(filename.as_ref())?;
    let fd = file.as_raw_fd();
    let watch = Watched {
      filter,
      flags,
      ident: Ident::Filename(file, fd, filename.as_ref().to_string_lossy().into_owned()),
    };

    if !self.watched.contains(&watch) {
      self.watched.insert(watch);
    }

    Ok(())
  }

  pub fn add_timer(&mut self, id: usize, dur: Duration) {
    let watch = Watched {
      filter: EventFilter::EVFILT_TIMER,
      flags: FilterFlag::NOTE_FFNOP,
      ident: Ident::Timer(id, dur),
    };

    if !self.watched.contains(&watch) {
      self.watched.insert(watch);
    }
  }

  /// Adds a descriptor to a `Watcher`. This or `add_file` is the preferred
  /// way to watch a file
  ///
  /// TODO: Adding new files requires calling `Watcher.watch` again
  pub fn add_fd(&mut self, fd: RawFd, filter: EventFilter, flags: FilterFlag) {
    let watch = Watched {
      filter,
      flags,
      ident: Ident::Fd(fd),
    };

    if !self.watched.contains(&watch) {
      self.watched.insert(watch);
    }
  }

  /// Adds a `File` to a `Watcher`. This, or `add_fd` is the preferred way
  /// to watch a file
  ///
  /// TODO: Adding new files requires calling `Watcher.watch` again
  pub fn add_file(&mut self, file: &File, filter: EventFilter, flags: FilterFlag) {
    self.add_fd(file.as_raw_fd(), filter, flags)
  }

  fn delete_kevents(&self, ident: Ident, filter: EventFilter) -> Result<()> {
    let kev = &[kevent::new(
      ident.as_usize(),
      filter,
      EventFlag::EV_DELETE,
      FilterFlag::empty(),
      0,
    )];

    match unsafe {
      kevent(
        self.queue,
        kev.as_ptr(),
        // On NetBSD, this is passed as a usize, not i32
        #[allow(clippy::useless_conversion)]
        i32::try_from(kev.len()).unwrap().try_into().unwrap(),
        ptr::null_mut(),
        0,
        ptr::null(),
      )
    } {
      -1 => Err(Error::last_os_error()),
      _ => Ok(()),
    }
  }

  /// Removes a pid from a `Watcher`
  pub fn remove_pid(&mut self, pid: libc::pid_t, filter: EventFilter) -> Result<bool> {
    if self.watched.is_empty() {
      return Ok(false);
    }

    let prev_len = self.watched.len();

    self.watched.retain(|w| w.ident != Ident::Pid(pid));

    match self.delete_kevents(Ident::Pid(pid), filter) {
      Ok(_) => Ok(self.watched.len() != prev_len),
      Err(err) => Err(err),
    }
  }

  /// Removes an fd from a `Watcher`. This closes the fd.
  pub fn remove_fd(&mut self, fd: RawFd, filter: EventFilter) -> Result<bool> {
    if self.watched.is_empty() {
      return Ok(false);
    }

    let prev_len = self.watched.len();

    self.watched.retain(|w| w.ident != Ident::Fd(fd));

    match self.delete_kevents(Ident::Fd(fd), filter) {
      Ok(_) => Ok(prev_len != self.watched.len()),
      Err(err) => Err(err),
    }
  }

  /// Removes a `File` from a `Watcher`
  pub fn remove_file(&mut self, file: &File, filter: EventFilter) -> Result<bool> {
    self.remove_fd(file.as_raw_fd(), filter)
  }

  /// Starts watching for events from `kqueue(2)`. This function needs to
  /// be called before `Watcher.iter()` or `Watcher.poll()` to actually
  /// start listening for events.
  pub fn watch(&mut self) -> Result<()> {
    let kevs: Vec<kevent> = self
      .watched
      .iter()
      .map(|watched| {
        let (raw_ident, data) = match watched.ident {
          Ident::Fd(fd) => (fd as uintptr_t, 0),
          Ident::Filename(_, fd, _) => (fd as uintptr_t, 0),
          Ident::Pid(pid) => (pid as uintptr_t, 0),
          Ident::Signal(sig) => (sig as uintptr_t, 0),
          Ident::Timer(ident, dur) => (
            ident as uintptr_t,
            (dur.as_secs() * 1000 + (dur.subsec_nanos() / 1_000_000) as u64) as i64,
          ),
        };

        kevent::new(
          raw_ident,
          watched.filter,
          if self.opts.clear {
            EventFlag::EV_ADD | EventFlag::EV_CLEAR
          } else {
            EventFlag::EV_ADD
          },
          watched.flags,
          data,
        )
      })
      .collect();

    let ret = unsafe {
      kevent(
        self.queue,
        kevs.as_ptr(),
        // On NetBSD, this is passed as a usize, not i32
        #[allow(clippy::useless_conversion)]
        i32::try_from(kevs.len()).unwrap().try_into().unwrap(),
        ptr::null_mut(),
        0,
        ptr::null(),
      )
    };

    self.started = true;
    match ret {
      -1 => Err(Error::last_os_error()),
      _ => Ok(()),
    }
  }

  /// Polls for a new event, with an optional timeout. If no `timeout`
  /// is passed, then it will return immediately.
  pub fn poll(&self, timeout: Option<Duration>) -> Option<Event> {
    // poll will not block indefinitely
    // None -> return immediately
    match timeout {
      Some(timeout) => get_event(self, Some(timeout)),
      None => get_event(self, Some(Duration::new(0, 0))),
    }
  }

  /// Polls for a new event, with an optional timeout. If no `timeout`
  /// is passed, then it will block until an event is received.
  pub fn poll_forever(&self, timeout: Option<Duration>) -> Option<Event> {
    if timeout.is_some() {
      self.poll(timeout)
    } else {
      get_event(self, None)
    }
  }

  /// Creates an iterator that iterates over the queue. This iterator will block
  /// until a new event is received.
  pub fn iter(&self) -> EventIter<'_> {
    EventIter { watcher: self }
  }
}

impl AsRawFd for Watcher {
  fn as_raw_fd(&self) -> RawFd {
    self.queue
  }
}

impl Drop for Watcher {
  fn drop(&mut self) {
    unsafe { libc::close(self.queue) };
    for watched in &self.watched {
      match watched.ident {
        Ident::Fd(fd) => unsafe { libc::close(fd) },
        Ident::Filename(_, fd, _) => unsafe { libc::close(fd) },
        _ => continue,
      };
    }
  }
}

fn get_event(watcher: &Watcher, timeout: Option<Duration>) -> Option<Event> {
  let mut kev = kevent::new(
    0,
    EventFilter::EVFILT_SYSCOUNT,
    EventFlag::empty(),
    FilterFlag::empty(),
    0,
  );

  let ret = if let Some(ts) = timeout {
    unsafe {
      kevent(
        watcher.queue,
        ptr::null(),
        0,
        &mut kev,
        1,
        &duration_to_timespec(ts),
      )
    }
  } else {
    unsafe { kevent(watcher.queue, ptr::null(), 0, &mut kev, 1, ptr::null()) }
  };

  match ret {
    -1 => Some(Event::from_error(kev, watcher)),
    0 => None, // timeout expired
    _ => Some(Event::new(kev, watcher)),
  }
}

impl Iterator for EventIter<'_> {
  type Item = Event;

  // rather than call kevent(2) each time, we can likely optimize and
  // call it once for like 100 items
  fn next(&mut self) -> Option<Self::Item> {
    if !self.watcher.started {
      return None;
    }

    get_event(self.watcher, None)
  }
}
