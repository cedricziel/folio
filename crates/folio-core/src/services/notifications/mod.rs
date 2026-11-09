//! Push notifications about processing outcomes

mod planner;
mod sender;
pub mod templates;

#[cfg(test)]
mod fakes;

pub use planner::NotificationPlanner;
pub use sender::{NotificationSender, SenderConfig, TestRefusal};
pub use templates::AlertSummary;
