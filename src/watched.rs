use std::hash::Hash;

use kqueue_sys::{EventFilter, FilterFlag};

use crate::Ident;

#[derive(Debug, PartialEq, Eq)]
pub struct Watched {
  pub(crate) filter: EventFilter,
  pub(crate) flags: FilterFlag,
  pub(crate) ident: Ident,
}

impl Hash for Watched {
  #[inline(always)]
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    state.write_usize(self.ident.as_usize())
  }
}
