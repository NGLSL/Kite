use crate::{CancelToken, Result};
use std::cell::RefCell;
use std::sync::atomic::Ordering;
thread_local! { static TOKEN: RefCell<Option<CancelToken>> = const { RefCell::new(None) }; }
pub(crate) struct Guard(Option<CancelToken>);
pub(crate) fn enter(token: CancelToken) -> Guard {
    Guard(TOKEN.with(|slot| slot.replace(Some(token))))
}
impl Drop for Guard {
    fn drop(&mut self) {
        TOKEN.with(|slot| slot.replace(self.0.take()));
    }
}

pub(crate) fn cancelled() -> bool {
    TOKEN.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|v| v.load(Ordering::Acquire))
    })
}
pub(crate) fn check() -> Result<()> {
    if cancelled() {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "file index operation cancelled",
        )
        .into())
    } else {
        Ok(())
    }
}
