use libc::{c_long, time_t, timespec};
use std::time::Duration;

#[cfg(all(target_arch = "x86_64", target_pointer_width = "32"))]
type NSec = i64;
#[cfg(not(all(target_arch = "x86_64", target_pointer_width = "32")))]
type NSec = c_long;

pub(crate) fn duration_to_timespec(d: Duration) -> timespec {
  let tv_sec = d.as_secs() as time_t;
  let tv_nsec = d.subsec_nanos() as NSec;

  if tv_sec.is_negative() {
    panic!("Duration seconds is negative");
  }

  if tv_nsec.is_negative() {
    panic!("Duration nsecs is negative");
  }

  timespec { tv_sec, tv_nsec }
}
