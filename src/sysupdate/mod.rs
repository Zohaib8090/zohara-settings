//! Operating-system updates, owned by Zohara Settings.
//!
//! Zohara Store handles apps; Settings handles the system. The two are separate programs that share no code: this is
//! Settings' own engine (check, update, undo, restore points, channel), with its own copy of the approval-list checker.
pub mod channel;
pub mod manifest;
pub mod ui;
pub mod updates;
