//! The bars around the panes: the window's own title bar, the top bar with the ribbon and the
//! search box, and the Activity button with its panel and the bar over the panes for what needs
//! you.

// The pieces name these as their siblings (`super::frame`), as they did beside them.
use super::{compacting, frame, importing};

pub(crate) mod activity;
pub(super) mod bars;
pub(crate) mod caption;
pub(super) mod ribbon;
pub(super) mod search_box;
pub(crate) mod send_receive;
